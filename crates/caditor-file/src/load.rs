use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::Arc,
};

use caditor_document::{Document, Edit, EditError, Feature, FeatureKind, Parameter, Transaction};
use caditor_expression::{Expression, ParameterId, check_name};

use crate::{
    binary::{self, History},
    format::{
        FEATURE_KINDS, FeatureRecord, NextIdsRecord, ParameterRecord, RECORD_KINDS, Record,
        Unreadable, restore_feature,
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
    #[error("this version is damaged and cannot be restored")]
    VersionUnavailable,
}

pub fn load(path: &Path) -> Result<Loaded, LoadError> {
    let bytes =
        std::fs::read(path).map_err(|error| LoadError::Unreadable(reason::reading(&error)))?;
    decode(&bytes)
}

pub fn decode(bytes: &[u8]) -> Result<Loaded, LoadError> {
    binary::decode(bytes)
}

pub fn history(path: &Path) -> Result<History, LoadError> {
    let bytes =
        std::fs::read(path).map_err(|error| LoadError::Unreadable(reason::reading(&error)))?;
    Ok(binary::history(&bytes))
}

pub fn load_version(path: &Path, index: usize) -> Result<Loaded, LoadError> {
    let bytes =
        std::fs::read(path).map_err(|error| LoadError::Unreadable(reason::reading(&error)))?;
    binary::load_version(&bytes, index)
}

pub(crate) fn newer_version(version: u32) -> String {
    format!(
        "This model was made by a newer version of caditor (format {version}). Anything this \
         version does not understand was left out."
    )
}

pub(crate) fn describe_unreadable_record(place: &str, item: &Unreadable<'_>) -> String {
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
            "{place} holds something this version of caditor does not know ({kind}), so it \
             was left out. It may come from a newer version."
        ),
        _ => format!("{place} is damaged and was left out."),
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
    pub(crate) fn remember_lost(&mut self, item: &Unreadable<'_>) {
        if item.kind() == Some("parameter")
            && let Some(id) = item.id()
        {
            self.lost_parameter_names
                .insert(id, item.name().unwrap_or_default().to_owned());
        }
    }

    pub(crate) fn add(&mut self, record: Record) {
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

    for mut feature in features {
        if document.features().any(|other| other.name == feature.name) {
            let renamed = unique_feature_name(&document, &feature.name);
            issues.push(format!(
                "Two features were named “{}”, so one of them is now “{renamed}”.",
                feature.name
            ));
            feature.name = renamed;
        }
        let name = feature.name.clone();
        let raw_id = feature.id().raw();
        let detached = detached(&feature);
        let on_datum = feature
            .kind
            .attachment()
            .is_some_and(|attachment| attachment.datum().is_some());
        let edit = Edit::InsertFeature {
            index: document.features().len(),
            feature: Arc::new(feature),
        };
        let inserted =
            match (document.apply(Transaction::single("Load", edit)), detached) {
                (Err(_), Some(detached)) => {
                    let edit = Edit::InsertFeature {
                        index: document.features().len(),
                        feature: Arc::new(detached),
                    };
                    let retried = document.apply(Transaction::single("Load", edit));
                    if retried.is_ok() {
                        issues.push(format!(
                        "“{name}” lay on {} that could not be restored, so the sketch now stays \
                         where it was.",
                        if on_datum { "a plane" } else { "a face of a body" }
                    ));
                    }
                    retried
                }
                (first, _) => first,
            };
        match inserted {
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

fn detached(feature: &Feature) -> Option<Feature> {
    let FeatureKind::Sketch(sketch) = &feature.kind else {
        return None;
    };
    sketch.attachment.as_ref()?;
    Some(Feature::new(
        feature.id(),
        feature.name.clone(),
        FeatureKind::from(sketch.sketch.clone()),
    ))
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

fn unique_feature_name(document: &Document, base: &str) -> String {
    (2_u32..)
        .map(|suffix| format!("{base} {suffix}"))
        .find(|candidate| document.features().all(|other| other.name != *candidate))
        .unwrap_or_else(|| base.to_owned())
}

fn unique_name(document: &Document, base: &str) -> String {
    std::iter::once(base.to_owned())
        .chain((2_u32..).map(|suffix| format!("{base}_{suffix}")))
        .find(|candidate| document.parameter_named(candidate).is_none())
        .unwrap_or_else(|| base.to_owned())
}
