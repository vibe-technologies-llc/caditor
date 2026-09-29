use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::Arc,
};

use ahash::{AHashMap, AHashSet};
use caditor_document::{
    DependencyGraph, Document, Edit, EditError, Feature, FeatureKind, Parameter, Revolve,
    RevolveAxis, SolidFeature, Transaction,
};
use caditor_expression::{Expression, ParameterId, check_name};
use caditor_sketch::EntityId;

use crate::{
    binary::{self, History},
    format::{
        FEATURE_KINDS, FeatureRecord, NextIdsRecord, ParameterRecord, RECORD_KINDS, Record,
        Unreadable, restore_feature,
    },
    read::read_file,
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
    let bytes = read_file(path).map_err(|error| LoadError::Unreadable(reason::reading(&error)))?;
    decode(&bytes)
}

pub fn decode(bytes: &[u8]) -> Result<Loaded, LoadError> {
    binary::decode(bytes)
}

pub fn history(path: &Path) -> Result<History, LoadError> {
    let bytes = read_file(path).map_err(|error| LoadError::Unreadable(reason::reading(&error)))?;
    Ok(binary::history(&bytes))
}

pub fn load_version(path: &Path, index: usize) -> Result<Loaded, LoadError> {
    let bytes = read_file(path).map_err(|error| LoadError::Unreadable(reason::reading(&error)))?;
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

pub const MAX_RECORDS: usize = 10_000;

#[derive(Debug, Clone, Default)]
pub(crate) struct Parts {
    pub parameters: Vec<ParameterRecord>,
    pub features: Vec<FeatureRecord>,
    pub next_ids: Option<NextIdsRecord>,
    pub lost_parameter_names: BTreeMap<u64, String>,
    pub beyond_limit: usize,
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
        let full = self.parameters.len() + self.features.len() >= MAX_RECORDS;
        match record {
            Record::Parameter(_) | Record::Feature(_) if full => self.beyond_limit += 1,
            Record::Parameter(parameter) => self.parameters.push(parameter),
            Record::Feature(feature) => self.features.push(*feature),
            Record::NextIds(next_ids) => self.next_ids = Some(next_ids),
        }
    }
}

#[derive(Default)]
struct TakenNames {
    names: AHashSet<String>,
    next_suffix: AHashMap<String, u32>,
}

impl TakenNames {
    fn contains(&self, name: &str) -> bool {
        self.names.contains(name)
    }

    fn take(&mut self, name: &str) {
        self.names.insert(name.to_owned());
    }

    fn numbered(&mut self, base: &str, separator: &str) -> String {
        let suffix = self.next_suffix.entry(base.to_owned()).or_insert(2);
        loop {
            let candidate = format!("{base}{separator}{suffix}");
            *suffix = suffix.saturating_add(1);
            if !self.names.contains(&candidate) {
                self.names.insert(candidate.clone());
                return candidate;
            }
        }
    }

    fn unique_parameter(&mut self, base: &str) -> String {
        if self.contains(base) {
            self.numbered(base, "_")
        } else {
            self.take(base);
            base.to_owned()
        }
    }
}

fn apply_each<T>(
    document: &mut Document,
    items: &[T],
    edit: &impl Fn(&Document, usize, &T) -> Edit,
    failed: &mut impl FnMut(&mut Document, &T, EditError),
) {
    let edits = items
        .iter()
        .enumerate()
        .map(|(offset, item)| edit(document, offset, item))
        .collect();
    let Err(error) = document.apply(Transaction::new("Load", edits)) else {
        return;
    };
    if let [item] = items {
        failed(document, item, error);
        return;
    }
    let (left, right) = items.split_at(items.len() / 2);
    apply_each(document, left, edit, failed);
    apply_each(document, right, edit, failed);
}

fn insert_parameters(
    document: &mut Document,
    parameters: &[Parameter],
    issues: &mut Vec<String>,
) -> AHashSet<ParameterId> {
    let mut failed = AHashSet::new();
    apply_each(
        document,
        parameters,
        &|document, offset, parameter| Edit::InsertParameter {
            index: document.parameters().len() + offset,
            parameter: parameter.clone(),
        },
        &mut |_, parameter, error| {
            issues.push(format!(
                "The parameter “{}” could not be restored and was left out: {error}.",
                parameter.name
            ));
            failed.insert(parameter.id());
        },
    );
    parameters
        .iter()
        .map(Parameter::id)
        .filter(|id| !failed.contains(id))
        .collect()
}

pub(crate) fn assemble(parts: Parts, issues: &mut Vec<String>) -> Document {
    let mut document = Document::default();
    if parts.beyond_limit > 0 {
        issues.push(format!(
            "The model holds more parameters and features than caditor loads ({MAX_RECORDS}), so \
             the last {} were left out.",
            parts.beyond_limit
        ));
    }

    let mut parameter_names = TakenNames::default();
    let mut seen = AHashSet::new();
    let mut read = Vec::new();
    for record in &parts.parameters {
        if !seen.insert(record.id) {
            issues.push(format!(
                "Two parameters share the ID {}, so “{}” was left out.",
                record.id, record.name
            ));
            continue;
        }
        let name = usable_name(&mut parameter_names, &record.name, record.id, issues);
        read.push((
            record,
            Parameter::new(
                ParameterId::from_raw(record.id),
                name,
                Expression::Number(0.0),
            ),
        ));
    }
    let placeholders: Vec<Parameter> = read
        .iter()
        .map(|(_, parameter)| parameter.clone())
        .collect();
    let mut inserted = insert_parameters(&mut document, &placeholders, issues);

    let mut expressions = Vec::new();
    for (record, parameter) in &read {
        if !inserted.contains(&parameter.id()) {
            continue;
        }
        match Expression::parse_stored(&record.expression) {
            Ok(expression) => {
                expressions.push((parameter.id(), parameter.name.clone(), expression))
            }
            Err(_) => issues.push(format!(
                "The value of “{}” could not be read, so it was set to 0. Enter its value again.",
                parameter.name
            )),
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
        .filter(|id| !inserted.contains(id))
        .collect();
    let stand_ins: Vec<Parameter> = referenced
        .into_iter()
        .map(|id| {
            let base = parts
                .lost_parameter_names
                .get(&id.raw())
                .filter(|name| check_name(name).is_ok())
                .cloned()
                .unwrap_or_else(|| format!("lost_{id}"));
            let name = parameter_names.unique_parameter(&base);
            Parameter::new(id, name, Expression::Number(0.0))
        })
        .collect();
    let stood_in = insert_parameters(&mut document, &stand_ins, issues);
    for stand_in in stand_ins
        .iter()
        .filter(|stand_in| stood_in.contains(&stand_in.id()))
    {
        issues.push(format!(
            "A parameter used elsewhere in the model could not be read. It was replaced by \
             “{}” = 0; enter its correct value.",
            stand_in.name
        ));
    }
    inserted.extend(stood_in);

    let expressions = without_cycles(&document, expressions, issues);
    apply_each(
        &mut document,
        &expressions,
        &|_, _, (id, _, expression)| Edit::SetParameterExpression {
            id: *id,
            expression: expression.clone(),
        },
        &mut |document, (_, name, expression), error| {
            let text = document.expression_text(expression);
            issues.push(format!(
                "The value of “{name}” could not be restored ({error}), so it was set to 0. Its \
                 value was {text}."
            ));
        },
    );

    let features = with_unique_names(features, issues);
    apply_each(
        &mut document,
        &features,
        &|document, offset, feature| Edit::InsertFeature {
            index: document.features().len() + offset,
            feature: Arc::clone(feature),
        },
        &mut |document, feature, error| insert_alone(document, feature, error, issues),
    );

    if let Some(next) = parts.next_ids {
        document.reserve_ids_below(next.parameter, next.feature);
    }
    document
}

fn with_unique_names(features: Vec<Feature>, issues: &mut Vec<String>) -> Vec<Arc<Feature>> {
    let mut names = TakenNames::default();
    let mut seen = AHashSet::new();
    let mut unique = Vec::with_capacity(features.len());
    for mut feature in features {
        if !seen.insert(feature.id()) {
            issues.push(format!(
                "Two features share the ID {}, so “{}” was left out.",
                feature.id().raw(),
                feature.name
            ));
            continue;
        }
        let trimmed = feature.name.trim();
        if !trimmed.is_empty() && trimmed.len() != feature.name.len() {
            feature.name = trimmed.to_owned();
        }
        if names.contains(&feature.name) {
            let renamed = names.numbered(&feature.name, " ");
            issues.push(format!(
                "Two features were named “{}”, so one of them is now “{renamed}”.",
                feature.name
            ));
            feature.name = renamed;
        } else {
            names.take(&feature.name);
        }
        unique.push(Arc::new(feature));
    }
    unique
}

fn without_cycles(
    document: &Document,
    expressions: Vec<(ParameterId, String, Expression)>,
    issues: &mut Vec<String>,
) -> Vec<(ParameterId, String, Expression)> {
    let names: AHashMap<ParameterId, &str> = document
        .parameters()
        .iter()
        .map(|parameter| (parameter.id(), parameter.name.as_str()))
        .collect();
    let mut graph = DependencyGraph::of(document);
    let mut acyclic = Vec::with_capacity(expressions.len());
    for (id, name, expression) in expressions {
        match graph.cycle(id, &expression) {
            Some(cycle) => {
                let path: Vec<&str> = cycle
                    .iter()
                    .map(|step| names.get(step).copied().unwrap_or("?"))
                    .collect();
                let text = document.expression_text(&expression);
                issues.push(format!(
                    "“{name}” depended on itself ({}), so it was set to 0. Its value was {text}.",
                    path.join(" → ")
                ));
            }
            None => {
                graph.set(id, &expression);
                acyclic.push((id, name, expression));
            }
        }
    }
    acyclic
}

fn insert_alone(
    document: &mut Document,
    feature: &Feature,
    error: EditError,
    issues: &mut Vec<String>,
) {
    let name = &feature.name;
    let inserted = match repaired(feature) {
        Some((repair, note)) => {
            let edit = Edit::InsertFeature {
                index: document.features().len(),
                feature: Arc::new(repair),
            };
            let retried = document.apply(Transaction::single("Load", edit));
            if retried.is_ok() {
                issues.push(note);
            }
            retried.map(|_| ())
        }
        None => Err(error),
    };
    match inserted {
        Ok(()) => {}
        Err(EditError::DuplicateId) => issues.push(format!(
            "Two features share the ID {}, so “{name}” was left out.",
            feature.id().raw()
        )),
        Err(error) => issues.push(format!(
            "“{name}” could not be restored and was left out: {error}."
        )),
    }
}

fn repaired(feature: &Feature) -> Option<(Feature, String)> {
    let (kind, reason) = repaired_kind(feature)?;
    let mut repaired = feature.clone();
    repaired.kind = kind;
    Some((repaired, reason))
}

fn repaired_kind(feature: &Feature) -> Option<(FeatureKind, String)> {
    let name = &feature.name;
    match &feature.kind {
        FeatureKind::Sketch(sketch) => {
            let attachment = sketch.attachment.as_ref()?;
            let lay_on = if attachment.datum().is_some() {
                "a plane"
            } else {
                "a face of a body"
            };
            Some((
                FeatureKind::from(sketch.sketch.clone()),
                format!(
                    "“{name}” lay on {lay_on} that could not be restored, so the sketch now stays \
                     where it was."
                ),
            ))
        }
        FeatureKind::Solid(SolidFeature::Revolve(revolve)) if matches!(revolve.axis, RevolveAxis::Sketch(line) if line != EntityId::VERTICAL_AXIS) =>
        {
            let turned = Revolve {
                axis: RevolveAxis::Sketch(EntityId::VERTICAL_AXIS),
                ..revolve.clone()
            };
            Some((
                FeatureKind::Solid(SolidFeature::Revolve(turned)),
                format!(
                    "“{name}” turned about a line of its sketch that could not be restored, so it \
                     now turns about the sketch's vertical axis."
                ),
            ))
        }
        _ => None,
    }
}

fn usable_name(names: &mut TakenNames, name: &str, id: u64, issues: &mut Vec<String>) -> String {
    let problem = match check_name(name) {
        Err(error) => Some(format!("“{name}” is not a usable name ({error})")),
        Ok(()) if names.contains(name) => Some(format!("two parameters are named “{name}”")),
        Ok(()) => None,
    };
    let Some(problem) = problem else {
        names.take(name);
        return name.to_owned();
    };
    let renamed = names.unique_parameter(&format!("parameter_{id}"));
    issues.push(format!(
        "A parameter was renamed to “{renamed}” because {problem}."
    ));
    renamed
}
