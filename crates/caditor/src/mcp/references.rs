use std::fmt;

use caditor_document::{Document, Evaluation, Feature, FeatureId, SolidResult};
use caditor_kernel::{FaceId, FaceName};
use caditor_sketch::{EntityId, Reference, Sketch};
use serde::Deserialize;

use crate::bodies::{self, FaceKey};

const FACE_KEY_DIGITS: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum Named {
    Id(u64),
    Name(String),
}

impl fmt::Display for Named {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Id(id) => write!(formatter, "with id {id}"),
            Self::Name(name) => write!(formatter, "“{name}”"),
        }
    }
}

impl Named {
    fn matches(&self, id: u64, name: &str) -> bool {
        match self {
            Self::Id(wanted) => *wanted == id,
            Self::Name(wanted) => wanted == name,
        }
    }

    fn as_id(&self) -> Option<u64> {
        match self {
            Self::Id(id) => Some(*id),
            Self::Name(name) => name.trim().parse().ok(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReferenceError {
    #[error("there is no feature {0}; list_features names every feature")]
    NoFeature(Named),
    #[error("{name} is not a sketch; get_sketch and sketch tools take a sketch")]
    NotASketch { name: String },
    #[error("there is no body {0}; list_bodies names every body")]
    NoBody(Named),
    #[error("“{0}” is not a face key; use one list_faces gives")]
    UnreadableFaceKey(String),
    #[error("{body} has no face {face} now; list_faces names its faces as they are")]
    NoFace { body: String, face: String },
    #[error(
        "{sketch} has no entity {entity}; get_sketch lists its entities, and origin, \
         horizontal_axis and vertical_axis name the sketch's own"
    )]
    NoEntity { sketch: String, entity: EntityName },
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum EntityName {
    Id(u64),
    Name(String),
}

impl fmt::Display for EntityName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Id(id) => write!(formatter, "{id}"),
            Self::Name(name) => write!(formatter, "“{name}”"),
        }
    }
}

pub fn feature<'d>(document: &'d Document, named: &Named) -> Result<&'d Feature, ReferenceError> {
    document
        .features()
        .find(|feature| named.matches(feature.id().raw(), &feature.name))
        .or_else(|| {
            named
                .as_id()
                .and_then(|id| document.feature(FeatureId::from_raw(id)))
        })
        .ok_or_else(|| ReferenceError::NoFeature(named.clone()))
}

pub fn sketch<'d>(
    document: &'d Document,
    named: &Named,
) -> Result<(&'d Feature, &'d Sketch), ReferenceError> {
    let found = feature(document, named)?;
    let sketch = found
        .kind
        .sketch()
        .ok_or_else(|| ReferenceError::NotASketch {
            name: found.name.clone(),
        })?;
    Ok((found, sketch))
}

pub fn body(
    document: &Document,
    evaluation: &Evaluation,
    named: &Named,
) -> Result<FeatureId, ReferenceError> {
    let standing: Vec<FeatureId> = evaluation.bodies().map(|(body, _)| body).collect();
    let label = |body: FeatureId| document.body_label(body).unwrap_or_default();
    standing
        .iter()
        .copied()
        .find(|body| named.matches(body.raw(), &label(*body)))
        .or_else(|| {
            let id = named.as_id()?;
            standing.iter().copied().find(|body| body.raw() == id)
        })
        .ok_or_else(|| ReferenceError::NoBody(named.clone()))
}

pub fn body_label(document: &Document, body: FeatureId) -> String {
    document
        .body_label(body)
        .unwrap_or_else(|| format!("body {}", body.raw()))
}

pub fn face_key_text(key: FaceKey) -> String {
    format!(
        "{:0width$x}:{}",
        key.name.digest(),
        key.occurrence,
        width = FACE_KEY_DIGITS
    )
}

pub fn face_key(text: &str) -> Result<FaceKey, ReferenceError> {
    let unreadable = || ReferenceError::UnreadableFaceKey(text.to_owned());
    let trimmed = text.trim();
    let (digits, occurrence) = match trimmed.split_once(':') {
        Some((digits, occurrence)) => (digits, occurrence.parse().map_err(|_| unreadable())?),
        None => (trimmed, 0),
    };
    if digits.is_empty() || digits.len() > FACE_KEY_DIGITS {
        return Err(unreadable());
    }
    let digest = u128::from_str_radix(digits, 16).map_err(|_| unreadable())?;
    Ok(FaceKey {
        name: FaceName::from_digest(digest),
        occurrence,
    })
}

pub fn face<'e>(
    document: &Document,
    evaluation: &'e Evaluation,
    body: FeatureId,
    text: &str,
) -> Result<(FaceKey, FaceId, &'e SolidResult), ReferenceError> {
    let key = face_key(text)?;
    let missing = || ReferenceError::NoFace {
        body: body_label(document, body),
        face: text.to_owned(),
    };
    let shown = bodies::shown(evaluation, body).ok_or_else(missing)?;
    let face = bodies::find_face(shown, key).ok_or_else(missing)?;
    Ok((key, face, shown))
}

pub fn entity(
    sketch_name: &str,
    sketch: &Sketch,
    named: &EntityName,
) -> Result<EntityId, ReferenceError> {
    let missing = || ReferenceError::NoEntity {
        sketch: sketch_name.to_owned(),
        entity: named.clone(),
    };
    let id = match named {
        EntityName::Id(id) => EntityId::from_raw(*id),
        EntityName::Name(name) => match reference_named(name) {
            Some(reference) => return Ok(reference.id()),
            None => EntityId::from_raw(name.trim().parse().map_err(|_| missing())?),
        },
    };
    if id.is_reference() || sketch.contains(id) {
        Ok(id)
    } else {
        Err(missing())
    }
}

pub fn entity_name(id: EntityId) -> serde_json::Value {
    match id.reference() {
        Some(reference) => serde_json::Value::from(reference_word(reference)),
        None => serde_json::Value::from(id.raw()),
    }
}

fn reference_named(name: &str) -> Option<Reference> {
    Reference::ALL
        .into_iter()
        .find(|reference| reference_word(*reference) == name.trim())
}

fn reference_word(reference: Reference) -> &'static str {
    match reference {
        Reference::Origin => "origin",
        Reference::HorizontalAxis => "horizontal_axis",
        Reference::VerticalAxis => "vertical_axis",
    }
}
