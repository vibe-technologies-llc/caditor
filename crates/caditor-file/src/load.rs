use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::Arc,
};

use caditor_document::{Document, Edit, EditError, Feature, Parameter, Transaction};
use caditor_expression::{Expression, ParameterId, check_name};

use crate::{
    format::{
        FEATURE_KINDS, FORMAT_NAME, FORMAT_VERSION, FeatureRecord, Header, Lenient, NextIdsRecord,
        ParameterRecord, RECORD_KINDS, Record, Unreadable, restore_feature,
    },
    reason,
};

#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    pub document: Document,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LoadError {
    #[error("{0}")]
    Unreadable(String),
    #[error("it is not a caditor model")]
    NotAModel,
    #[error("the file is empty")]
    Empty,
}

pub fn load(path: &Path) -> Result<Loaded, LoadError> {
    let bytes =
        std::fs::read(path).map_err(|error| LoadError::Unreadable(reason::reading(&error)))?;
    decode(&bytes)
}

pub fn decode(bytes: &[u8]) -> Result<Loaded, LoadError> {
    let mut issues = Vec::new();
    let mut parts = Parts::default();
    let mut header = None;
    let mut records_read = 0_usize;

    let lines = bytes
        .split(|byte| *byte == b'\n')
        .enumerate()
        .map(|(index, line)| (index + 1, String::from_utf8_lossy(line)))
        .filter(|(_, line)| !line.trim().is_empty());
    for (number, line) in lines {
        let line = line.trim();
        if header.is_none() {
            match serde_json::from_str::<Header>(line) {
                Ok(read) if read.format == FORMAT_NAME => {
                    if read.version > FORMAT_VERSION {
                        issues.push(format!(
                            "This model was made by a newer version of caditor (format {}). \
                             Anything this version does not understand was left out.",
                            read.version
                        ));
                    }
                    header = Some(HeaderState::Read);
                    continue;
                }
                Ok(_) => return Err(LoadError::NotAModel),
                Err(_) => header = Some(HeaderState::Damaged { line: number }),
            }
        }
        match serde_json::from_str::<Lenient<Record>>(line) {
            Ok(Lenient::Read(record)) => {
                records_read += 1;
                parts.add(record);
            }
            Ok(Lenient::Unreadable(value)) => {
                let item = Unreadable(&value);
                if item.kind() == Some("parameter")
                    && let Some(id) = item.id()
                {
                    parts
                        .lost_parameter_names
                        .insert(id, item.name().unwrap_or_default().to_owned());
                }
                issues.push(describe_unreadable_record(number, &item));
            }
            Err(_) if header == Some(HeaderState::Damaged { line: number }) => {}
            Err(_) => issues.push(format!("Line {number} is damaged and was left out.")),
        }
    }

    match header {
        Some(HeaderState::Read) => {}
        Some(HeaderState::Damaged { .. }) if records_read > 0 => issues.insert(
            0,
            format!(
                "The start of the file is damaged; the rest was read as a version \
                 {FORMAT_VERSION} model."
            ),
        ),
        Some(HeaderState::Damaged { .. }) => return Err(LoadError::NotAModel),
        None => return Err(LoadError::Empty),
    }

    let document = assemble(parts, &mut issues);
    Ok(Loaded { document, issues })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeaderState {
    Read,
    Damaged { line: usize },
}

fn describe_unreadable_record(line: usize, item: &Unreadable<'_>) -> String {
    let name = item.name();
    match (item.kind(), name) {
        (Some("feature"), Some(name)) => match item.unknown_kind(&FEATURE_KINDS) {
            Some(kind) => format!(
                "The feature “{name}” is a kind this version of caditor does not know ({kind}), \
                 so it was left out. It may come from a newer version."
            ),
            None => format!("The feature “{name}” is damaged and was left out."),
        },
        (Some("parameter"), Some(name)) => {
            format!("The parameter “{name}” is damaged and was left out.")
        }
        (Some(kind), _) if !RECORD_KINDS.contains(&kind) => format!(
            "Line {line} holds something this version of caditor does not know ({kind}), so it \
             was left out. It may come from a newer version."
        ),
        _ => format!("Line {line} is damaged and was left out."),
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Parts {
    pub parameters: Vec<ParameterRecord>,
    pub features: Vec<FeatureRecord>,
    pub next_ids: Option<NextIdsRecord>,
    pub lost_parameter_names: BTreeMap<u64, String>,
}

impl Parts {
    fn add(&mut self, record: Record) {
        match record {
            Record::Parameter(parameter) => self.parameters.push(parameter),
            Record::Feature(feature) => self.features.push(feature),
            Record::NextIds(next_ids) => self.next_ids = Some(next_ids),
        }
    }
}

pub(crate) fn assemble(parts: Parts, issues: &mut Vec<String>) -> Document {
    let mut document = Document::default();
    let mut expressions = Vec::new();

    for record in &parts.parameters {
        let id = ParameterId::from_raw(record.id);
        if document.parameter(id).is_some() {
            issues.push(format!(
                "Two parameters share the ID {}, so “{}” was left out.",
                record.id, record.name
            ));
            continue;
        }
        let name = usable_name(&document, &record.name, record.id, issues);
        if insert_placeholder(&mut document, id, name.clone(), issues) {
            match Expression::parse_stored(&record.expression) {
                Ok(expression) => expressions.push((id, name, expression)),
                Err(_) => issues.push(format!(
                    "The value of “{name}” could not be read, so it was set to 0. Enter its \
                     value again."
                )),
            }
        }
    }

    let features: Vec<Feature> = parts
        .features
        .iter()
        .map(|record| restore_feature(record, issues))
        .collect();

    let referenced: BTreeSet<ParameterId> = expressions
        .iter()
        .flat_map(|(_, _, expression)| expression.parameters())
        .chain(
            features
                .iter()
                .flat_map(|feature| feature.kind.parameters()),
        )
        .collect();
    for id in referenced {
        if document.parameter(id).is_some() {
            continue;
        }
        let base = parts
            .lost_parameter_names
            .get(&id.raw())
            .filter(|name| check_name(name).is_ok())
            .cloned()
            .unwrap_or_else(|| format!("lost_{id}"));
        let name = unique_name(&document, &base);
        if insert_placeholder(&mut document, id, name.clone(), issues) {
            issues.push(format!(
                "A parameter used elsewhere in the model could not be read. It was replaced by \
                 “{name}” = 0; enter its correct value."
            ));
        }
    }

    for (id, name, expression) in expressions {
        let text = document.expression_text(&expression);
        let edit = Edit::SetParameterExpression { id, expression };
        match document.apply(Transaction::single("Load", edit)) {
            Ok(_) => {}
            Err(EditError::Cycle { path, .. }) => issues.push(format!(
                "“{name}” depended on itself ({path}), so it was set to 0. Its value was {text}."
            )),
            Err(error) => issues.push(format!(
                "The value of “{name}” could not be restored ({error}), so it was set to 0. Its \
                 value was {text}."
            )),
        }
    }

    for feature in features {
        let name = feature.name.clone();
        let raw_id = feature.id().raw();
        let edit = Edit::InsertFeature {
            index: document.features().len(),
            feature: Arc::new(feature),
        };
        match document.apply(Transaction::single("Load", edit)) {
            Ok(_) => {}
            Err(EditError::DuplicateId) => issues.push(format!(
                "Two features share the ID {raw_id}, so “{name}” was left out."
            )),
            Err(error) => issues.push(format!(
                "“{name}” could not be restored and was left out: {error}."
            )),
        }
    }

    if let Some(next) = parts.next_ids {
        document.reserve_ids_below(next.parameter, next.feature);
    }
    document
}

fn insert_placeholder(
    document: &mut Document,
    id: ParameterId,
    name: String,
    issues: &mut Vec<String>,
) -> bool {
    let edit = Edit::InsertParameter {
        index: document.parameters().len(),
        parameter: Parameter::new(id, name.clone(), Expression::Number(0.0)),
    };
    match document.apply(Transaction::single("Load", edit)) {
        Ok(_) => true,
        Err(error) => {
            issues.push(format!(
                "The parameter “{name}” could not be restored and was left out: {error}."
            ));
            false
        }
    }
}

fn usable_name(document: &Document, name: &str, id: u64, issues: &mut Vec<String>) -> String {
    let problem = match check_name(name) {
        Err(error) => Some(format!("“{name}” is not a usable name ({error})")),
        Ok(()) if document.parameter_named(name).is_some() => {
            Some(format!("two parameters are named “{name}”"))
        }
        Ok(()) => None,
    };
    let Some(problem) = problem else {
        return name.to_owned();
    };
    let renamed = unique_name(document, &format!("parameter_{id}"));
    issues.push(format!(
        "A parameter was renamed to “{renamed}” because {problem}."
    ));
    renamed
}

fn unique_name(document: &Document, base: &str) -> String {
    std::iter::once(base.to_owned())
        .chain((2_u32..).map(|suffix| format!("{base}_{suffix}")))
        .find(|candidate| document.parameter_named(candidate).is_none())
        .unwrap_or_else(|| base.to_owned())
}
