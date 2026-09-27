use std::sync::Arc;

use caditor_document::{Document, Edit, Feature, FeatureId, FeatureKind, Parameter, Transaction};
use caditor_expression::{Expression, ParameterId, Quantity, Unit};
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_sketch::{Constraint, ConstraintId, Entity, EntityId, Sketch};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::DeserializeOwned};
use serde_json::Value;

pub const FORMAT_VERSION: u32 = 2;
pub(crate) const FORMAT_NAME: &str = "caditor";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Header {
    pub format: String,
    pub version: u32,
}

impl Header {
    pub fn current() -> Self {
        Self {
            format: FORMAT_NAME.to_owned(),
            version: FORMAT_VERSION,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Record {
    Parameter(ParameterRecord),
    Feature(FeatureRecord),
    NextIds(NextIdsRecord),
}

pub(crate) const RECORD_KINDS: [&str; 3] = ["parameter", "feature", "next_ids"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ParameterRecord {
    pub id: u64,
    pub name: String,
    pub expression: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct FeatureRecord {
    pub id: u64,
    pub name: String,
    #[serde(flatten)]
    pub kind: FeatureKindRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FeatureKindRecord {
    Sketch(SketchRecord),
}

pub(crate) const FEATURE_KINDS: [&str; 1] = ["sketch"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct SketchRecord {
    pub plane: PlaneRecord,
    pub entities: Vec<Lenient<EntityRecord>>,
    pub constraints: Vec<Lenient<ConstraintRecord>>,
    pub next_id: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(crate) struct PlaneRecord {
    pub origin: [f64; 3],
    pub normal: [f64; 3],
    pub x_axis: [f64; 3],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct EntityRecord {
    pub id: u64,
    #[serde(flatten)]
    pub kind: EntityKindRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EntityKindRecord {
    Point([f64; 2]),
    Line { start: u64, end: u64 },
    Circle { center: u64, radius: f64 },
    Arc { center: u64, start: u64, end: u64 },
    Spline { control_points: Vec<u64> },
}

const ENTITY_KINDS: [&str; 5] = ["point", "line", "circle", "arc", "spline"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ConstraintRecord {
    pub id: u64,
    #[serde(flatten)]
    pub kind: ConstraintKindRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ConstraintKindRecord {
    Coincident([u64; 2]),
    Horizontal(u64),
    Vertical(u64),
    Parallel([u64; 2]),
    Perpendicular([u64; 2]),
    Tangent([u64; 2]),
    Equal([u64; 2]),
    Distance { from: u64, to: u64, value: String },
    Angle { from: u64, to: u64, value: String },
    Radius { entity: u64, value: String },
}

const CONSTRAINT_KINDS: [&str; 10] = [
    "coincident",
    "horizontal",
    "vertical",
    "parallel",
    "perpendicular",
    "tangent",
    "equal",
    "distance",
    "angle",
    "radius",
];

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(crate) struct NextIdsRecord {
    pub parameter: u64,
    pub feature: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct TransactionRecord {
    pub label: String,
    pub edits: Vec<EditRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EditRecord {
    InsertParameter {
        index: usize,
        parameter: ParameterRecord,
    },
    RemoveParameter {
        id: u64,
    },
    RenameParameter {
        id: u64,
        name: String,
    },
    SetParameterExpression {
        id: u64,
        expression: String,
    },
    InsertFeature {
        index: usize,
        feature: FeatureRecord,
    },
    RemoveFeature {
        id: u64,
    },
    RenameFeature {
        id: u64,
        name: String,
    },
    MoveFeature {
        id: u64,
        index: usize,
    },
    SetDimension {
        feature: u64,
        constraint: u64,
        value: String,
    },
    AddSketchEntity {
        feature: u64,
        entity: EntityRecord,
    },
    RemoveSketchEntity {
        feature: u64,
        id: u64,
    },
    SetSketchEntity {
        feature: u64,
        entity: EntityRecord,
    },
    AddSketchConstraint {
        feature: u64,
        constraint: ConstraintRecord,
    },
    RemoveSketchConstraint {
        feature: u64,
        id: u64,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Lenient<T> {
    Read(T),
    Unreadable(Value),
}

impl<T: Serialize> Serialize for Lenient<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Read(read) => read.serialize(serializer),
            Self::Unreadable(value) => value.serialize(serializer),
        }
    }
}

impl<'de, T: DeserializeOwned> Deserialize<'de> for Lenient<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Ok(match T::deserialize(&value) {
            Ok(read) => Self::Read(read),
            Err(error) => {
                log::warn!("could not read {value}: {error}");
                Self::Unreadable(value)
            }
        })
    }
}

pub(crate) struct Unreadable<'a>(pub &'a Value);

impl Unreadable<'_> {
    pub fn kind(&self) -> Option<&str> {
        match self.0 {
            Value::Object(fields) if fields.len() == 1 => fields.keys().next().map(String::as_str),
            _ => None,
        }
    }

    pub fn body(&self) -> &Value {
        match self.0 {
            Value::Object(fields) if fields.len() == 1 => fields.values().next().unwrap_or(self.0),
            _ => self.0,
        }
    }

    pub fn name(&self) -> Option<&str> {
        self.body().get("name").and_then(Value::as_str)
    }

    pub fn id(&self) -> Option<u64> {
        self.body().get("id").and_then(Value::as_u64)
    }

    pub fn unknown_kind(&self, known: &[&str]) -> Option<&str> {
        unknown_kind(self.body(), known)
    }
}

pub(crate) fn unknown_kind<'a>(value: &'a Value, known: &[&str]) -> Option<&'a str> {
    let Value::Object(fields) = value else {
        return None;
    };
    fields
        .keys()
        .map(String::as_str)
        .find(|key| *key != "id" && *key != "name" && !known.contains(key))
}

pub(crate) fn parameter_record(parameter: &Parameter) -> ParameterRecord {
    ParameterRecord {
        id: parameter.id().raw(),
        name: parameter.name.clone(),
        expression: parameter.expression.to_stored_text(),
    }
}

pub(crate) fn feature_record(feature: &Feature) -> FeatureRecord {
    FeatureRecord {
        id: feature.id().raw(),
        name: feature.name.clone(),
        kind: match &feature.kind {
            FeatureKind::Sketch(sketch) => FeatureKindRecord::Sketch(sketch_record(sketch)),
        },
    }
}

fn sketch_record(sketch: &Sketch) -> SketchRecord {
    let plane = sketch.plane();
    SketchRecord {
        plane: PlaneRecord {
            origin: plane.origin().to_array(),
            normal: plane.normal().to_array(),
            x_axis: plane.x_axis().to_array(),
        },
        entities: sketch
            .entities()
            .map(|(id, entity)| Lenient::Read(entity_record(id, entity)))
            .collect(),
        constraints: sketch
            .constraints()
            .map(|(id, constraint)| Lenient::Read(constraint_record(id, constraint)))
            .collect(),
        next_id: sketch.next_id(),
    }
}

fn entity_record(id: EntityId, entity: &Entity) -> EntityRecord {
    EntityRecord {
        id: id.raw(),
        kind: entity_kind_record(entity),
    }
}

fn constraint_record(id: ConstraintId, constraint: &Constraint) -> ConstraintRecord {
    ConstraintRecord {
        id: id.raw(),
        kind: constraint_kind_record(constraint),
    }
}

fn entity_kind_record(entity: &Entity) -> EntityKindRecord {
    match entity {
        Entity::Point(position) => EntityKindRecord::Point(position.to_array()),
        Entity::Line { start, end } => EntityKindRecord::Line {
            start: start.raw(),
            end: end.raw(),
        },
        Entity::Circle { center, radius } => EntityKindRecord::Circle {
            center: center.raw(),
            radius: *radius,
        },
        Entity::Arc { center, start, end } => EntityKindRecord::Arc {
            center: center.raw(),
            start: start.raw(),
            end: end.raw(),
        },
        Entity::Spline { control_points } => EntityKindRecord::Spline {
            control_points: control_points.iter().map(|point| point.raw()).collect(),
        },
    }
}

fn constraint_kind_record(constraint: &Constraint) -> ConstraintKindRecord {
    let pair = |a: &EntityId, b: &EntityId| [a.raw(), b.raw()];
    match constraint {
        Constraint::Coincident(a, b) => ConstraintKindRecord::Coincident(pair(a, b)),
        Constraint::Horizontal(entity) => ConstraintKindRecord::Horizontal(entity.raw()),
        Constraint::Vertical(entity) => ConstraintKindRecord::Vertical(entity.raw()),
        Constraint::Parallel(a, b) => ConstraintKindRecord::Parallel(pair(a, b)),
        Constraint::Perpendicular(a, b) => ConstraintKindRecord::Perpendicular(pair(a, b)),
        Constraint::Tangent(a, b) => ConstraintKindRecord::Tangent(pair(a, b)),
        Constraint::Equal(a, b) => ConstraintKindRecord::Equal(pair(a, b)),
        Constraint::Distance { from, to, value } => ConstraintKindRecord::Distance {
            from: from.raw(),
            to: to.raw(),
            value: value.to_stored_text(),
        },
        Constraint::Angle { from, to, value } => ConstraintKindRecord::Angle {
            from: from.raw(),
            to: to.raw(),
            value: value.to_stored_text(),
        },
        Constraint::Radius { entity, value } => ConstraintKindRecord::Radius {
            entity: entity.raw(),
            value: value.to_stored_text(),
        },
    }
}

pub(crate) fn next_ids_record(document: &Document) -> NextIdsRecord {
    NextIdsRecord {
        parameter: document.next_parameter_id(),
        feature: document.next_feature_id(),
    }
}

pub(crate) fn transaction_record(transaction: &Transaction) -> TransactionRecord {
    TransactionRecord {
        label: transaction.label().to_owned(),
        edits: transaction.edits().iter().map(edit_record).collect(),
    }
}

fn edit_record(edit: &Edit) -> EditRecord {
    match edit {
        Edit::InsertParameter { index, parameter } => EditRecord::InsertParameter {
            index: *index,
            parameter: parameter_record(parameter),
        },
        Edit::RemoveParameter { id } => EditRecord::RemoveParameter { id: id.raw() },
        Edit::RenameParameter { id, name } => EditRecord::RenameParameter {
            id: id.raw(),
            name: name.clone(),
        },
        Edit::SetParameterExpression { id, expression } => EditRecord::SetParameterExpression {
            id: id.raw(),
            expression: expression.to_stored_text(),
        },
        Edit::InsertFeature { index, feature } => EditRecord::InsertFeature {
            index: *index,
            feature: feature_record(feature),
        },
        Edit::RemoveFeature { id } => EditRecord::RemoveFeature { id: id.raw() },
        Edit::RenameFeature { id, name } => EditRecord::RenameFeature {
            id: id.raw(),
            name: name.clone(),
        },
        Edit::MoveFeature { id, index } => EditRecord::MoveFeature {
            id: id.raw(),
            index: *index,
        },
        Edit::SetDimension {
            feature,
            constraint,
            value,
        } => EditRecord::SetDimension {
            feature: feature.raw(),
            constraint: constraint.raw(),
            value: value.to_stored_text(),
        },
        Edit::AddSketchEntity {
            feature,
            id,
            entity,
        } => EditRecord::AddSketchEntity {
            feature: feature.raw(),
            entity: entity_record(*id, entity),
        },
        Edit::RemoveSketchEntity { feature, id } => EditRecord::RemoveSketchEntity {
            feature: feature.raw(),
            id: id.raw(),
        },
        Edit::SetSketchEntity {
            feature,
            id,
            entity,
        } => EditRecord::SetSketchEntity {
            feature: feature.raw(),
            entity: entity_record(*id, entity),
        },
        Edit::AddSketchConstraint {
            feature,
            id,
            constraint,
        } => EditRecord::AddSketchConstraint {
            feature: feature.raw(),
            constraint: constraint_record(*id, constraint),
        },
        Edit::RemoveSketchConstraint { feature, id } => EditRecord::RemoveSketchConstraint {
            feature: feature.raw(),
            id: id.raw(),
        },
    }
}

pub(crate) fn restore_transaction(record: TransactionRecord) -> Option<Transaction> {
    let edits = record
        .edits
        .into_iter()
        .map(restore_edit)
        .collect::<Option<Vec<_>>>()?;
    Some(Transaction::new(record.label, edits))
}

fn restore_edit(record: EditRecord) -> Option<Edit> {
    let parse = |text: &str| Expression::parse_stored(text).ok();
    Some(match record {
        EditRecord::InsertParameter { index, parameter } => Edit::InsertParameter {
            index,
            parameter: Parameter::new(
                ParameterId::from_raw(parameter.id),
                parameter.name,
                parse(&parameter.expression)?,
            ),
        },
        EditRecord::RemoveParameter { id } => Edit::RemoveParameter {
            id: ParameterId::from_raw(id),
        },
        EditRecord::RenameParameter { id, name } => Edit::RenameParameter {
            id: ParameterId::from_raw(id),
            name,
        },
        EditRecord::SetParameterExpression { id, expression } => Edit::SetParameterExpression {
            id: ParameterId::from_raw(id),
            expression: parse(&expression)?,
        },
        EditRecord::InsertFeature { index, feature } => {
            let mut issues = Vec::new();
            let feature = restore_feature(&feature, &mut issues);
            if !issues.is_empty() {
                return None;
            }
            Edit::InsertFeature {
                index,
                feature: Arc::new(feature),
            }
        }
        EditRecord::RemoveFeature { id } => Edit::RemoveFeature {
            id: FeatureId::from_raw(id),
        },
        EditRecord::RenameFeature { id, name } => Edit::RenameFeature {
            id: FeatureId::from_raw(id),
            name,
        },
        EditRecord::MoveFeature { id, index } => Edit::MoveFeature {
            id: FeatureId::from_raw(id),
            index,
        },
        EditRecord::SetDimension {
            feature,
            constraint,
            value,
        } => Edit::SetDimension {
            feature: FeatureId::from_raw(feature),
            constraint: ConstraintId::from_raw(constraint),
            value: parse(&value)?,
        },
        EditRecord::AddSketchEntity { feature, entity } => Edit::AddSketchEntity {
            feature: FeatureId::from_raw(feature),
            id: EntityId::from_raw(entity.id),
            entity: restore_entity(&entity.kind),
        },
        EditRecord::RemoveSketchEntity { feature, id } => Edit::RemoveSketchEntity {
            feature: FeatureId::from_raw(feature),
            id: EntityId::from_raw(id),
        },
        EditRecord::SetSketchEntity { feature, entity } => Edit::SetSketchEntity {
            feature: FeatureId::from_raw(feature),
            id: EntityId::from_raw(entity.id),
            entity: restore_entity(&entity.kind),
        },
        EditRecord::AddSketchConstraint {
            feature,
            constraint,
        } => Edit::AddSketchConstraint {
            feature: FeatureId::from_raw(feature),
            id: ConstraintId::from_raw(constraint.id),
            constraint: constraint_from_record(&constraint.kind, |text, _| parse(text))?,
        },
        EditRecord::RemoveSketchConstraint { feature, id } => Edit::RemoveSketchConstraint {
            feature: FeatureId::from_raw(feature),
            id: ConstraintId::from_raw(id),
        },
    })
}

pub(crate) fn restore_feature(record: &FeatureRecord, issues: &mut Vec<String>) -> Feature {
    let name = if record.name.trim().is_empty() {
        let fallback = format!("Feature {}", record.id);
        issues.push(format!(
            "A feature had no name, so it was named “{fallback}”."
        ));
        fallback
    } else {
        record.name.clone()
    };
    let kind = match &record.kind {
        FeatureKindRecord::Sketch(sketch) => {
            FeatureKind::Sketch(restore_sketch(sketch, &name, issues))
        }
    };
    Feature::new(FeatureId::from_raw(record.id), name, kind)
}

fn restore_sketch(record: &SketchRecord, feature: &str, issues: &mut Vec<String>) -> Sketch {
    let plane = restore_plane(record.plane).unwrap_or_else(|| {
        issues.push(format!(
            "The plane of “{feature}” could not be read, so the sketch was placed on the XY \
             plane."
        ));
        Plane::XY
    });
    let mut sketch = Sketch::new(plane);

    let mut readable = Vec::new();
    for entity in &record.entities {
        match entity {
            Lenient::Read(entity) => readable.push(entity),
            Lenient::Unreadable(value) => {
                issues.push(unreadable_item(feature, value, &ENTITY_KINDS, "an entity"))
            }
        }
    }
    let (points, others): (Vec<_>, Vec<_>) = readable
        .into_iter()
        .partition(|entity| matches!(entity.kind, EntityKindRecord::Point(_)));
    for record in points.into_iter().chain(others) {
        let entity = restore_entity(&record.kind);
        let label = format!("{} {}", entity.kind_name(), record.id);
        if let Err(error) = sketch.insert_entity(EntityId::from_raw(record.id), entity) {
            issues.push(format!(
                "In “{feature}”, {label} was left out because {error}."
            ));
        }
    }

    for constraint in &record.constraints {
        match constraint {
            Lenient::Read(record) => restore_constraint(&mut sketch, record, feature, issues),
            Lenient::Unreadable(value) => issues.push(unreadable_item(
                feature,
                value,
                &CONSTRAINT_KINDS,
                "a constraint",
            )),
        }
    }
    sketch.reserve_ids_below(record.next_id);
    sketch
}

fn restore_entity(record: &EntityKindRecord) -> Entity {
    let entity = EntityId::from_raw;
    match record {
        EntityKindRecord::Point([x, y]) => Entity::Point(Point2::new(*x, *y)),
        EntityKindRecord::Line { start, end } => Entity::Line {
            start: entity(*start),
            end: entity(*end),
        },
        EntityKindRecord::Circle { center, radius } => Entity::Circle {
            center: entity(*center),
            radius: *radius,
        },
        EntityKindRecord::Arc { center, start, end } => Entity::Arc {
            center: entity(*center),
            start: entity(*start),
            end: entity(*end),
        },
        EntityKindRecord::Spline { control_points } => Entity::Spline {
            control_points: control_points.iter().copied().map(entity).collect(),
        },
    }
}

fn restore_plane(record: PlaneRecord) -> Option<Plane> {
    Plane::from_frame(
        Point3::from_array(record.origin),
        Vector3::from_array(record.normal),
        Vector3::from_array(record.x_axis),
    )
}

fn restore_constraint(
    sketch: &mut Sketch,
    record: &ConstraintRecord,
    feature: &str,
    issues: &mut Vec<String>,
) {
    let constraint = constraint_from_record(&record.kind, |text, kind| {
        restore_dimension(sketch, text, kind, feature, issues)
    });
    let Some(constraint) = constraint else {
        return;
    };
    if let Err(error) = sketch.insert_constraint(ConstraintId::from_raw(record.id), constraint) {
        issues.push(format!(
            "In “{feature}”, a constraint was left out because {error}."
        ));
    }
}

fn constraint_from_record(
    record: &ConstraintKindRecord,
    mut value: impl FnMut(&str, DrawnValue) -> Option<Expression>,
) -> Option<Constraint> {
    let entity = EntityId::from_raw;
    let pair = |[a, b]: [u64; 2]| (entity(a), entity(b));
    Some(match record {
        ConstraintKindRecord::Coincident(ids) => {
            let (a, b) = pair(*ids);
            Constraint::Coincident(a, b)
        }
        ConstraintKindRecord::Horizontal(line) => Constraint::Horizontal(entity(*line)),
        ConstraintKindRecord::Vertical(line) => Constraint::Vertical(entity(*line)),
        ConstraintKindRecord::Parallel(ids) => {
            let (a, b) = pair(*ids);
            Constraint::Parallel(a, b)
        }
        ConstraintKindRecord::Perpendicular(ids) => {
            let (a, b) = pair(*ids);
            Constraint::Perpendicular(a, b)
        }
        ConstraintKindRecord::Tangent(ids) => {
            let (a, b) = pair(*ids);
            Constraint::Tangent(a, b)
        }
        ConstraintKindRecord::Equal(ids) => {
            let (a, b) = pair(*ids);
            Constraint::Equal(a, b)
        }
        ConstraintKindRecord::Distance {
            from,
            to,
            value: text,
        } => {
            let (from, to) = (entity(*from), entity(*to));
            let value = value(text, DrawnValue::Distance { from, to })?;
            Constraint::Distance { from, to, value }
        }
        ConstraintKindRecord::Angle {
            from,
            to,
            value: text,
        } => {
            let (from, to) = (entity(*from), entity(*to));
            let value = value(text, DrawnValue::Angle { from, to })?;
            Constraint::Angle { from, to, value }
        }
        ConstraintKindRecord::Radius {
            entity: curve,
            value: text,
        } => {
            let curve = entity(*curve);
            let value = value(text, DrawnValue::Radius(curve))?;
            Constraint::Radius {
                entity: curve,
                value,
            }
        }
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DrawnValue {
    Distance { from: EntityId, to: EntityId },
    Angle { from: EntityId, to: EntityId },
    Radius(EntityId),
}

impl DrawnValue {
    fn noun(self) -> &'static str {
        match self {
            Self::Distance { .. } => "a distance",
            Self::Angle { .. } => "an angle",
            Self::Radius(_) => "a radius",
        }
    }

    fn drawn_name(self) -> &'static str {
        match self {
            Self::Distance { .. } => "drawn length",
            Self::Angle { .. } => "drawn angle",
            Self::Radius(_) => "drawn radius",
        }
    }

    fn measure(self, sketch: &Sketch) -> Option<Quantity> {
        match self {
            Self::Distance { from, to } => {
                let point_distance = sketch
                    .point(from)
                    .zip(sketch.point(to))
                    .map(|(a, b)| a.distance(b));
                point_distance
                    .or_else(|| line_distance(sketch, from, to))
                    .or_else(|| line_distance(sketch, to, from))
                    .map(Quantity::length)
            }
            Self::Angle { from, to } => {
                let (from, to) = (sketch.line_direction(from)?, sketch.line_direction(to)?);
                let radians = from.perp_dot(to).atan2(from.dot(to));
                Some(Quantity::angle(radians.to_degrees()))
            }
            Self::Radius(curve) => sketch
                .circle(curve)
                .map(|(_, radius)| radius)
                .filter(|radius| *radius > 0.0)
                .map(Quantity::length),
        }
        .filter(|quantity| quantity.value.is_finite())
    }
}

fn line_distance(sketch: &Sketch, point: EntityId, line: EntityId) -> Option<f64> {
    let position = sketch.point(point)?;
    let direction = sketch.line_direction(line)?.try_normalize()?;
    let anchor = match sketch.line_endpoints(line) {
        Some((start, _)) => start,
        None => Point2::ZERO,
    };
    Some(direction.perp_dot(position - anchor).abs())
}

fn restore_dimension(
    sketch: &Sketch,
    text: &str,
    kind: DrawnValue,
    feature: &str,
    issues: &mut Vec<String>,
) -> Option<Expression> {
    if let Ok(value) = Expression::parse_stored(text) {
        return Some(value);
    }
    let noun = kind.noun();
    let Some(drawn) = kind.measure(sketch) else {
        issues.push(format!(
            "In “{feature}”, {noun} could not be read and was left out."
        ));
        return None;
    };
    issues.push(format!(
        "In “{feature}”, the value of {noun} could not be read, so it was set to its {}, \
         {drawn}.",
        kind.drawn_name()
    ));
    let unit = if matches!(kind, DrawnValue::Angle { .. }) {
        Unit::Degree
    } else {
        Unit::Millimetre
    };
    Some(Expression::Measure(drawn.value, unit))
}

fn unreadable_item(feature: &str, item: &Value, known: &[&str], noun: &str) -> String {
    match unknown_kind(item, known) {
        Some(kind) => format!(
            "In “{feature}”, {noun} of a kind this version of caditor does not know ({kind}) was \
             left out. It may come from a newer version."
        ),
        None => format!("In “{feature}”, {noun} was damaged and was left out."),
    }
}
