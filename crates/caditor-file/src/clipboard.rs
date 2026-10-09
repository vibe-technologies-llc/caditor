use std::fmt;

use caditor_document::{
    CarriedParameter, CarriedParameters, Document, Feature, FeatureId, PasteOrigin, literal,
};
use caditor_expression::{Expression, ParameterId, Quantity};
use caditor_geometry::{Plane, Vector2};
use caditor_sketch::{ClipError, EntityId, Sketch, SketchClip};
use serde::{Deserialize, Serialize};

use crate::format::{
    ConstraintRecord, EntityRecord, FeatureRecord, Lenient, SketchRecord, Unreadable,
    constraint_record, entity_record, feature_record, plane_record, restore_feature,
    restore_sketch,
};

pub const CLIPBOARD_HEADER: &str = "caditor clipboard";
pub const CLIPBOARD_VERSION: u32 = 1;
pub const MAX_CLIPBOARD_TEXT: usize = 16 << 20;

const KIND_SEPARATOR: &str = ": ";
const VERSION_SEPARATOR: &str = ", version ";
const PASTED_GEOMETRY: &str = "the pasted geometry";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardKind {
    SketchGeometry,
    Features,
}

impl ClipboardKind {
    const ALL: [Self; 2] = [Self::SketchGeometry, Self::Features];

    fn word(self) -> &'static str {
        match self {
            Self::SketchGeometry => "sketch geometry",
            Self::Features => "features",
        }
    }

    fn header(self) -> String {
        format!(
            "{CLIPBOARD_HEADER}{KIND_SEPARATOR}{}{VERSION_SEPARATOR}{CLIPBOARD_VERSION}",
            self.word()
        )
    }
}

impl fmt::Display for ClipboardKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.word())
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ClipboardError {
    #[error("the clipboard holds text that did not come from caditor")]
    Foreign,
    #[error("the clipboard holds {found} from caditor, not {expected}")]
    OtherKind {
        found: ClipboardKind,
        expected: ClipboardKind,
    },
    #[error("the clipboard holds something from caditor of a kind this version does not know")]
    UnknownKind,
    #[error(
        "the clipboard holds {kind} from a newer caditor (clipboard version {version}); update \
         caditor to paste it"
    )]
    Newer { kind: ClipboardKind, version: u32 },
    #[error("the copied {kind} is damaged and cannot be read")]
    Damaged { kind: ClipboardKind },
    #[error("the clipboard text is too large to paste ({length} bytes)")]
    TooLarge { length: usize },
    #[error("the copied sketch geometry holds nothing that can be pasted")]
    NothingUsable,
    #[error("the copy could not be written as text")]
    Unwritable,
    #[error(transparent)]
    Clip(#[from] ClipError),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct CarriedParameterRecord {
    id: u64,
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct SketchClipRecord {
    source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sketch: Option<u64>,
    entities: Vec<Lenient<EntityRecord>>,
    constraints: Vec<Lenient<ConstraintRecord>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    parameters: Vec<Lenient<CarriedParameterRecord>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct FeaturesRecord {
    source: String,
    features: Vec<Lenient<FeatureRecord>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    suppressed: Vec<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    parameters: Vec<Lenient<CarriedParameterRecord>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PastedGeometry {
    pub clip: SketchClip,
    pub origin: PasteOrigin,
    pub sketch: Option<FeatureId>,
    pub inlined: usize,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CopiedFeatures {
    pub features: Vec<Feature>,
    pub parameters: CarriedParameters,
    pub origin: PasteOrigin,
    pub notes: Vec<String>,
}

pub fn clipboard_kind(text: &str) -> Result<ClipboardKind, ClipboardError> {
    let (kind, _) = split_header(text)?;
    Ok(kind)
}

pub fn sketch_clipboard_text(
    clip: &SketchClip,
    parameters: &CarriedParameters,
    source: &str,
    sketch: FeatureId,
) -> Result<String, ClipboardError> {
    let mut holder = Sketch::new(Plane::XY);
    holder.paste(clip, Vector2::ZERO)?;
    let record = SketchClipRecord {
        source: source.to_owned(),
        sketch: Some(sketch.raw()),
        entities: holder
            .entities()
            .filter(|(id, _)| !id.is_reference())
            .map(|(id, entity)| {
                Lenient::Read(entity_record(id, entity, holder.is_construction(id)))
            })
            .collect(),
        constraints: holder
            .constraints()
            .map(|(id, constraint)| {
                Lenient::Read(constraint_record(
                    id,
                    constraint,
                    !holder.is_active(id),
                    None,
                ))
            })
            .collect(),
        parameters: parameter_records(parameters),
    };
    payload(ClipboardKind::SketchGeometry, &record)
}

pub fn read_sketch_clipboard(
    text: &str,
    target: &Document,
    own_source: &str,
) -> Result<PastedGeometry, ClipboardError> {
    let kind = ClipboardKind::SketchGeometry;
    let record: SketchClipRecord = read_payload(text, kind)?;
    let origin = origin_of(&record.source, own_source);
    let mut notes = Vec::new();
    let parameters = restore_parameters(&record.parameters, &mut notes);
    let stored = SketchRecord {
        plane: plane_record(Plane::XY),
        attachment: None,
        datum: None,
        entities: record.entities,
        constraints: record.constraints,
        projections: Vec::new(),
        next_id: 0,
    };
    let mut sketch = restore_sketch(&stored, PASTED_GEOMETRY, &mut notes);
    let mut inlined = 0;
    let dimensions: Vec<_> = sketch
        .constraints()
        .filter_map(|(id, constraint)| Some((id, constraint.dimension()?.clone())))
        .collect();
    for (id, value) in dimensions {
        match parameters.carry(&value, target, origin) {
            Ok(carried) => {
                inlined += carried.inlined;
                if carried.expression != value {
                    sketch
                        .set_dimension(id, carried.expression)
                        .map_err(ClipError::from)?;
                }
            }
            Err(error) => {
                sketch.remove_constraint(id).map_err(ClipError::from)?;
                notes.push(format!("A dimension was left out because {error}."));
            }
        }
    }
    let items: Vec<EntityId> = sketch
        .entities()
        .map(|(id, _)| id)
        .filter(|id| !id.is_reference())
        .collect();
    let clip = sketch
        .clip(&items)
        .map_err(|_| ClipboardError::NothingUsable)?;
    Ok(PastedGeometry {
        clip,
        origin,
        sketch: record.sketch.map(FeatureId::from_raw),
        inlined,
        notes,
    })
}

pub fn features_clipboard_text(
    features: &[Feature],
    parameters: &CarriedParameters,
    source: &str,
) -> Result<String, ClipboardError> {
    let record = FeaturesRecord {
        source: source.to_owned(),
        features: features
            .iter()
            .map(|feature| Lenient::Read(feature_record(feature)))
            .collect(),
        suppressed: features
            .iter()
            .filter(|feature| feature.suppressed)
            .map(|feature| feature.id().raw())
            .collect(),
        parameters: parameter_records(parameters),
    };
    payload(ClipboardKind::Features, &record)
}

pub fn read_features_clipboard(
    text: &str,
    own_source: &str,
) -> Result<CopiedFeatures, ClipboardError> {
    let record: FeaturesRecord = read_payload(text, ClipboardKind::Features)?;
    let mut notes = Vec::new();
    let parameters = restore_parameters(&record.parameters, &mut notes);
    let mut features = Vec::new();
    for feature in &record.features {
        match feature {
            Lenient::Read(read) => {
                let mut restored = restore_feature(read, &mut notes);
                restored.suppressed = record.suppressed.contains(&read.id);
                features.push(restored);
            }
            Lenient::Unreadable(value) => {
                let unreadable = Unreadable(value);
                notes.push(match unreadable.name() {
                    Some(name) => format!(
                        "“{name}” was left out because it is damaged or of a kind this version \
                         of caditor does not know."
                    ),
                    None => "A copied feature was left out because it is damaged or of a kind \
                             this version of caditor does not know."
                        .to_owned(),
                });
            }
        }
    }
    Ok(CopiedFeatures {
        features,
        parameters,
        origin: origin_of(&record.source, own_source),
        notes,
    })
}

fn origin_of(source: &str, own_source: &str) -> PasteOrigin {
    if source == own_source {
        PasteOrigin::ThisDocument
    } else {
        PasteOrigin::Elsewhere
    }
}

fn payload(kind: ClipboardKind, record: &impl Serialize) -> Result<String, ClipboardError> {
    let body = serde_json::to_string(record).map_err(|_| ClipboardError::Unwritable)?;
    Ok(format!("{}\n{body}\n", kind.header()))
}

fn split_header(text: &str) -> Result<(ClipboardKind, &str), ClipboardError> {
    let text = text.trim_start_matches('\u{feff}');
    let (header, body) = text.split_once('\n').unwrap_or((text, ""));
    let described = header
        .trim_end_matches('\r')
        .strip_prefix(CLIPBOARD_HEADER)
        .and_then(|rest| rest.strip_prefix(KIND_SEPARATOR))
        .ok_or(ClipboardError::Foreign)?;
    let (word, version) = described
        .rsplit_once(VERSION_SEPARATOR)
        .ok_or(ClipboardError::UnknownKind)?;
    let kind = ClipboardKind::ALL
        .into_iter()
        .find(|kind| kind.word() == word)
        .ok_or(ClipboardError::UnknownKind)?;
    let version: u32 = version
        .trim()
        .parse()
        .map_err(|_| ClipboardError::Damaged { kind })?;
    if version > CLIPBOARD_VERSION {
        return Err(ClipboardError::Newer { kind, version });
    }
    Ok((kind, body))
}

fn read_payload<T: for<'de> Deserialize<'de>>(
    text: &str,
    expected: ClipboardKind,
) -> Result<T, ClipboardError> {
    if text.len() > MAX_CLIPBOARD_TEXT {
        return Err(ClipboardError::TooLarge { length: text.len() });
    }
    let (found, body) = split_header(text)?;
    if found != expected {
        return Err(ClipboardError::OtherKind { found, expected });
    }
    serde_json::from_str(body.trim()).map_err(|_| ClipboardError::Damaged { kind: expected })
}

fn parameter_records(parameters: &CarriedParameters) -> Vec<Lenient<CarriedParameterRecord>> {
    parameters
        .iter()
        .map(|(id, parameter)| {
            Lenient::Read(CarriedParameterRecord {
                id: id.raw(),
                name: parameter.name.clone(),
                value: parameter
                    .value
                    .and_then(literal)
                    .map(|value| value.to_stored_text()),
            })
        })
        .collect()
}

fn restore_parameters(
    records: &[Lenient<CarriedParameterRecord>],
    notes: &mut Vec<String>,
) -> CarriedParameters {
    let mut parameters = CarriedParameters::default();
    for record in records {
        match record {
            Lenient::Read(record) => parameters.insert(
                ParameterId::from_raw(record.id),
                CarriedParameter {
                    name: record.name.clone(),
                    value: record.value.as_deref().and_then(literal_value),
                },
            ),
            Lenient::Unreadable(_) => notes.push(
                "A copied parameter was damaged, so values using it were left out.".to_owned(),
            ),
        }
    }
    parameters
}

fn literal_value(text: &str) -> Option<Quantity> {
    let expression = Expression::parse_stored(text).ok()?;
    if !expression.parameters().is_empty() {
        return None;
    }
    expression
        .evaluate(&|_| Err(caditor_expression::EvalError::ParameterMissing))
        .ok()
}

#[cfg(test)]
mod tests;
