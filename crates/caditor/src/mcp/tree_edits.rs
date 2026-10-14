use caditor_document::{Document, Edit, EditError, Feature, FeatureId, Transaction, TreeRow};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::mcp::{
    references::{self, Named},
    session::{Session, Work},
    tools::{Answer, ToolError},
    writing::apply,
};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Renaming {
    pub feature: Named,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Suppressing {
    pub features: Vec<Named>,
    #[serde(default = "suppressed_by_default")]
    pub suppressed: bool,
}

fn suppressed_by_default() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reordering {
    pub feature: Named,
    pub before: Option<Named>,
    pub after: Option<Named>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dependents {
    Delete,
    Keep,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Deleting {
    pub features: Vec<Named>,
    pub dependents: Option<Dependents>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum TreeError {
    #[error("give at least one feature")]
    NoFeatures,
    #[error("say where {feature} goes: before or after another feature")]
    NoPlace { feature: String },
    #[error("give before or after, not both")]
    BothPlaces,
    #[error("{feature} cannot go there: {source}")]
    Unmovable {
        feature: String,
        #[source]
        source: Box<EditError>,
    },
    #[error(
        "{dependents} depend on {subject}; say dependents: \"delete\" to delete them as well, or \
         \"keep\" to leave them failing in the tree"
    )]
    HasDependents { subject: String, dependents: String },
}

fn names(document: &Document, ids: &[FeatureId]) -> String {
    let named: Vec<String> = ids
        .iter()
        .filter_map(|id| document.feature(*id))
        .map(|feature| format!("“{}”", feature.name))
        .collect();
    match named.as_slice() {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn chosen(document: &Document, features: &[Named]) -> Result<Vec<FeatureId>, ToolError> {
    if features.is_empty() {
        return Err(TreeError::NoFeatures.into());
    }
    let mut ids = Vec::with_capacity(features.len());
    for named in features {
        let id = references::feature(document, named)?.id();
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    Ok(ids)
}

fn listed(document: &Document, ids: &[FeatureId]) -> Value {
    Value::Array(
        ids.iter()
            .filter_map(|id| document.feature(*id))
            .map(|feature| json!({ "id": feature.id().raw(), "name": feature.name }))
            .collect(),
    )
}

pub fn rename(
    session: &mut Session,
    work: &Work<'_>,
    asked: Renaming,
) -> Result<Answer, ToolError> {
    let document = session.document();
    let feature = references::feature(document, &asked.feature)?;
    let transaction = Transaction::single(
        format!("Rename {}", feature.name),
        Edit::RenameFeature {
            id: feature.id(),
            name: asked.name.clone(),
        },
    );
    let mut details = Map::new();
    details.insert("feature".to_owned(), json!(feature.id().raw()));
    details.insert("name".to_owned(), json!(asked.name));
    apply(session, work, transaction, details)
}

pub fn suppress(
    session: &mut Session,
    work: &Work<'_>,
    asked: Suppressing,
) -> Result<Answer, ToolError> {
    let document = session.document();
    let ids = chosen(document, &asked.features)?;
    let verb = if asked.suppressed {
        "Suppress"
    } else {
        "Unsuppress"
    };
    let transaction = document.suppression(
        &ids,
        asked.suppressed,
        format!("{verb} {}", names(document, &ids)),
    );
    let mut details = Map::new();
    details.insert("features".to_owned(), listed(document, &ids));
    details.insert("suppressed".to_owned(), json!(asked.suppressed));
    apply(session, work, transaction, details)
}

fn row_of(rows: &[TreeRow], id: FeatureId) -> Option<usize> {
    rows.iter().position(|row| *row == TreeRow::Feature(id))
}

pub fn reorder(
    session: &mut Session,
    work: &Work<'_>,
    asked: Reordering,
) -> Result<Answer, ToolError> {
    let document = session.document();
    let moved: &Feature = references::feature(document, &asked.feature)?;
    let rows = document.tree_rows();
    let gap = match (&asked.before, &asked.after) {
        (Some(_), Some(_)) => return Err(TreeError::BothPlaces.into()),
        (None, None) => {
            return Err(TreeError::NoPlace {
                feature: moved.name.clone(),
            }
            .into());
        }
        (Some(before), None) => {
            let anchor = references::feature(document, before)?.id();
            row_of(&rows, anchor)
        }
        (None, Some(after)) => {
            let anchor = references::feature(document, after)?.id();
            row_of(&rows, anchor).map(|row| row + 1)
        }
    };
    let gap = gap.unwrap_or(rows.len());
    let transaction = document
        .move_row(
            TreeRow::Feature(moved.id()),
            gap,
            format!("Move {}", moved.name),
        )
        .map_err(|source| TreeError::Unmovable {
            feature: moved.name.clone(),
            source: Box::new(source),
        })?;
    let mut details = Map::new();
    details.insert("feature".to_owned(), json!(moved.id().raw()));
    let answer = apply(session, work, transaction, details)?;
    let mut data = answer.data;
    let document = session.document();
    let order: Vec<Value> = document
        .features()
        .map(|feature| json!({ "id": feature.id().raw(), "name": feature.name }))
        .collect();
    data.insert("tree".to_owned(), Value::Array(order));
    Ok(Answer::of(data))
}

pub fn delete(
    session: &mut Session,
    work: &Work<'_>,
    asked: Deleting,
) -> Result<Answer, ToolError> {
    let document = session.document();
    let ids = chosen(document, &asked.features)?;
    let dependents = document.dependents_of(&ids);
    let subject = names(document, &ids);
    let (doomed, label) = match (asked.dependents, dependents.is_empty()) {
        (_, true) | (Some(Dependents::Keep), false) => (ids.clone(), format!("Delete {subject}")),
        (Some(Dependents::Delete), false) => {
            let whose = if ids.len() == 1 { "its" } else { "their" };
            (
                ids.iter().chain(&dependents).copied().collect(),
                format!("Delete {subject} and {whose} dependents"),
            )
        }
        (None, false) => {
            return Err(TreeError::HasDependents {
                subject,
                dependents: names(document, &dependents),
            }
            .into());
        }
    };
    let transaction = document.deletion(&doomed, label);
    let mut details = Map::new();
    details.insert("deleted".to_owned(), listed(document, &doomed));
    apply(session, work, transaction, details)
}
