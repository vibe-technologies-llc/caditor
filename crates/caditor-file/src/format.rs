use std::sync::Arc;

use caditor_document::{
    AxisReference, Blend, BlendKind, BodyOperation, CircularPattern, Datum, DatumAxis, DatumPlane,
    Document, Edit, Extrude, ExtrudeEnd, ExtrudeExtent, FaceAttachment, Feature, FeatureId,
    FeatureKind, Import, LinearDirection, Parameter, Pattern, PatternKind, PlaneReference,
    PlaneRotation, PrincipalAxis, PrincipalGeometry, PrincipalPlane, RegionChoice, Revolve,
    RevolveAxis, RevolveExtent, RollbackBar, Shell, SketchAttachment, SketchFeature, SolidFeature,
    Transaction,
};
use caditor_expression::{Expression, ParameterId, Quantity, Unit};
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_kernel::{
    BoundaryPiece, EdgeName, EdgeReference, FaceCopy, FaceName, FaceOrigin, FaceReference,
    RegionKey, RegionReference, Side, Solid, VertexName,
};
use caditor_sketch::{Constraint, ConstraintId, Entity, EntityId, Sketch};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::DeserializeOwned};
use serde_json::Value;

pub const FORMAT_VERSION: u32 = 3;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Record {
    Parameter(ParameterRecord),
    Feature(Box<FeatureRecord>),
    NextIds(NextIdsRecord),
    Principal(PrincipalRecord),
    Suppressed(SuppressedRecord),
    Rollback(RollbackRecord),
}

pub(crate) const RECORD_KINDS: [&str; 6] = [
    "parameter",
    "feature",
    "next_ids",
    "principal",
    "suppressed",
    "rollback",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct SuppressedRecord {
    pub features: Vec<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(crate) struct RollbackRecord {
    pub before: u64,
}

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
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    #[serde(flatten)]
    pub kind: FeatureKindRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FeatureKindRecord {
    Sketch(SketchRecord),
    Extrude(ExtrudeRecord),
    ExtrudeTo(ExtrudeToRecord),
    Revolve(RevolveRecord),
    RevolveTwoAngles(RevolveTwoAnglesRecord),
    Fillet(BlendRecord),
    Chamfer(BlendRecord),
    Shell(ShellRecord),
    LinearPattern(Box<LinearPatternRecord>),
    CircularPattern(Box<CircularPatternRecord>),
    Plane(Box<DatumPlaneRecord>),
    Axis(Box<DatumAxisRecord>),
    Import(ImportRecord),
}

pub(crate) const FEATURE_KINDS: [&str; 13] = [
    "sketch",
    "extrude",
    "extrude_to",
    "revolve",
    "revolve_two_angles",
    "fillet",
    "chamfer",
    "shell",
    "linear_pattern",
    "circular_pattern",
    "plane",
    "axis",
    "import",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ImportRecord {
    pub source: String,
    pub step: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PrincipalPlaneRecord {
    Xy,
    Xz,
    Yz,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PrincipalAxisRecord {
    X,
    Y,
    Z,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PrincipalGeometryRecord {
    Origin,
    Axis(PrincipalAxisRecord),
    Plane(PrincipalPlaneRecord),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct PrincipalRecord {
    pub hidden: Vec<PrincipalGeometryRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PlaneReferenceRecord {
    Principal(PrincipalPlaneRecord),
    Datum(u64),
    Face(AttachmentRecord),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AxisReferenceRecord {
    Principal(PrincipalAxisRecord),
    Datum(u64),
    Edge { body: u64, edge: EdgeRecord },
    Face { body: u64, face: FaceRecord },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct RotationRecord {
    pub axis: AxisReferenceRecord,
    pub angle: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct DatumPlaneRecord {
    pub base: PlaneReferenceRecord,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<RotationRecord>,
    pub offset: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DatumAxisRecord {
    Along(AxisReferenceRecord),
    Intersection([PlaneReferenceRecord; 2]),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum RevolveAxisRecord {
    Sketch(u64),
    Model(Box<AxisReferenceRecord>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ShellRecord {
    pub body: u64,
    pub thickness: String,
    pub open: Vec<Lenient<FaceRecord>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct DirectionRecord {
    pub axis: AxisReferenceRecord,
    pub count: String,
    pub spacing: String,
    pub reversed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct LinearPatternRecord {
    pub body: u64,
    pub first: DirectionRecord,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub second: Option<DirectionRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct CircularPatternRecord {
    pub body: u64,
    pub axis: AxisReferenceRecord,
    pub count: String,
    pub angle: String,
    pub reversed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct FaceRecord {
    pub face: String,
    pub origin: Option<FaceOriginRecord>,
    pub neighbours: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copy: Option<CopyRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(crate) struct CopyRecord {
    pub pattern: u64,
    pub index: [u32; 2],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct BlendRecord {
    pub body: u64,
    pub size: String,
    pub edges: Vec<Lenient<EdgeRecord>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct EdgeRecord {
    pub name: String,
    pub faces: [String; 2],
    pub ends: [String; 2],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origins: Option<[Option<FaceOriginRecord>; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copies: Option<[Option<CopyRecord>; 2]>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RegionsRecord {
    All,
    Chosen(Vec<String>),
}

type RegionReferencesRecord = Option<Vec<Lenient<RegionReferenceRecord>>>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct RegionReferenceRecord {
    pub boundary: Vec<BoundaryPieceRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<[f64; 2]>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct BoundaryPieceRecord {
    pub entity: u64,
    pub side: SideRecord,
    pub piece: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SideRecord {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OperationRecord {
    NewBody,
    Add(u64),
    Remove(u64),
    Intersect(u64),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExtrudeExtentRecord {
    OneSide { distance: String, reversed: bool },
    Symmetric { distance: String },
    TwoSides { forward: String, backward: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RevolveExtentRecord {
    Full,
    OneSide { angle: String, reversed: bool },
    Symmetric { angle: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ExtrudeRecord {
    pub sketch: u64,
    pub regions: RegionsRecord,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region_references: RegionReferencesRecord,
    pub extent: ExtrudeExtentRecord,
    pub operation: OperationRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExtrudeEndRecord {
    Distance(String),
    ThroughAll,
    UpToNext,
    UpToFace(PlaneReferenceRecord),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExtrudeEndsRecord {
    OneSide {
        end: Lenient<ExtrudeEndRecord>,
        reversed: bool,
    },
    TwoSides {
        forward: Lenient<ExtrudeEndRecord>,
        backward: Lenient<ExtrudeEndRecord>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ExtrudeToRecord {
    pub sketch: u64,
    pub regions: RegionsRecord,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region_references: RegionReferencesRecord,
    pub extent: ExtrudeEndsRecord,
    pub operation: OperationRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct RevolveTwoAnglesRecord {
    pub sketch: u64,
    pub regions: RegionsRecord,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region_references: RegionReferencesRecord,
    pub axis: RevolveAxisRecord,
    pub forward: String,
    pub backward: String,
    pub operation: OperationRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct RevolveRecord {
    pub sketch: u64,
    pub regions: RegionsRecord,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region_references: RegionReferencesRecord,
    pub axis: RevolveAxisRecord,
    pub extent: RevolveExtentRecord,
    pub operation: OperationRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct SketchRecord {
    pub plane: PlaneRecord,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment: Option<Lenient<AttachmentRecord>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub datum: Option<u64>,
    pub entities: Vec<Lenient<EntityRecord>>,
    pub constraints: Vec<Lenient<ConstraintRecord>>,
    pub next_id: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct AttachmentRecord {
    pub body: u64,
    pub face: String,
    pub origin: Option<FaceOriginRecord>,
    pub neighbours: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copy: Option<CopyRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FaceOriginRecord {
    Side { feature: u64, entity: u64 },
    StartCap { feature: u64 },
    EndCap { feature: u64 },
    Fillet { feature: u64 },
    Chamfer { feature: u64 },
    Shell { feature: u64 },
    Imported { feature: u64, face: u32 },
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
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub construction: bool,
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
    Distance {
        from: u64,
        to: u64,
        value: String,
    },
    Angle {
        from: u64,
        to: u64,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        reversed: bool,
        value: String,
    },
    Radius {
        entity: u64,
        value: String,
    },
    HorizontalPoints([u64; 2]),
    VerticalPoints([u64; 2]),
    Midpoint {
        point: u64,
        line: u64,
    },
    Concentric([u64; 2]),
    Collinear([u64; 2]),
    Symmetric {
        first: u64,
        second: u64,
        about: u64,
    },
    Fix {
        point: u64,
        at: [f64; 2],
    },
    HorizontalDistance {
        from: u64,
        to: u64,
        value: String,
    },
    VerticalDistance {
        from: u64,
        to: u64,
        value: String,
    },
    Diameter {
        entity: u64,
        value: String,
    },
}

const CONSTRAINT_KINDS: [&str; 20] = [
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
    "horizontal_points",
    "vertical_points",
    "midpoint",
    "concentric",
    "collinear",
    "symmetric",
    "fix",
    "horizontal_distance",
    "vertical_distance",
    "diameter",
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
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        suppressed: bool,
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
    SetFeatureHidden {
        id: u64,
        hidden: bool,
    },
    SetFeatureSuppressed {
        id: u64,
        suppressed: bool,
    },
    SetRollbackBar {
        before: Option<u64>,
    },
    SetPrincipalHidden {
        geometry: PrincipalGeometryRecord,
        hidden: bool,
    },
    SetFeatureKind {
        feature: FeatureRecord,
    },
    SetSketchPlacement {
        feature: u64,
        plane: PlaneRecord,
        attachment: Option<AttachmentRecord>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        datum: Option<u64>,
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
    SetSketchConstruction {
        feature: u64,
        id: u64,
        construction: bool,
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
        hidden: feature.hidden,
        kind: feature_kind_record(&feature.kind),
    }
}

fn feature_kind_record(kind: &FeatureKind) -> FeatureKindRecord {
    match kind {
        FeatureKind::Sketch(sketch) => FeatureKindRecord::Sketch(sketch_record(sketch)),
        FeatureKind::Solid(SolidFeature::Extrude(extrude)) => extrude_record(extrude),
        FeatureKind::Solid(SolidFeature::Revolve(revolve)) => revolve_record(revolve),
        FeatureKind::Blend(blend) => {
            let record = BlendRecord {
                body: blend.body.raw(),
                size: blend.size.to_stored_text(),
                edges: blend
                    .edges
                    .iter()
                    .map(|edge| Lenient::Read(edge_record(edge)))
                    .collect(),
            };
            match blend.kind {
                BlendKind::Fillet => FeatureKindRecord::Fillet(record),
                BlendKind::Chamfer => FeatureKindRecord::Chamfer(record),
            }
        }
        FeatureKind::Datum(Datum::Plane(plane)) => {
            FeatureKindRecord::Plane(Box::new(DatumPlaneRecord {
                base: plane_reference_record(&plane.base),
                rotation: plane.rotation.as_ref().map(|rotation| RotationRecord {
                    axis: axis_record(&rotation.axis),
                    angle: rotation.angle.to_stored_text(),
                }),
                offset: plane.offset.to_stored_text(),
            }))
        }
        FeatureKind::Datum(Datum::Axis(axis)) => FeatureKindRecord::Axis(Box::new(match axis {
            DatumAxis::Along(reference) => DatumAxisRecord::Along(axis_record(reference)),
            DatumAxis::Intersection(first, second) => DatumAxisRecord::Intersection([
                plane_reference_record(first),
                plane_reference_record(second),
            ]),
        })),
        FeatureKind::Shell(shell) => FeatureKindRecord::Shell(ShellRecord {
            body: shell.body.raw(),
            thickness: shell.thickness.to_stored_text(),
            open: shell
                .open
                .iter()
                .map(|face| Lenient::Read(face_record(face)))
                .collect(),
        }),
        FeatureKind::Pattern(pattern) => pattern_record(pattern),
        FeatureKind::Import(import) => FeatureKindRecord::Import(ImportRecord {
            source: import.source.clone(),
            step: import.step.to_string(),
        }),
    }
}

fn end_record(end: &ExtrudeEnd) -> Lenient<ExtrudeEndRecord> {
    Lenient::Read(match end {
        ExtrudeEnd::Distance(distance) => ExtrudeEndRecord::Distance(distance.to_stored_text()),
        ExtrudeEnd::ThroughAll => ExtrudeEndRecord::ThroughAll,
        ExtrudeEnd::UpToNext => ExtrudeEndRecord::UpToNext,
        ExtrudeEnd::UpToFace(target) => ExtrudeEndRecord::UpToFace(plane_reference_record(target)),
    })
}

fn extrude_record(extrude: &Extrude) -> FeatureKindRecord {
    let distances = |extent: ExtrudeExtentRecord| {
        FeatureKindRecord::Extrude(ExtrudeRecord {
            sketch: extrude.sketch.raw(),
            regions: regions_record(&extrude.regions),
            region_references: region_references_record(&extrude.regions),
            extent,
            operation: operation_record(extrude.operation),
        })
    };
    let ends = |extent: ExtrudeEndsRecord| {
        FeatureKindRecord::ExtrudeTo(ExtrudeToRecord {
            sketch: extrude.sketch.raw(),
            regions: regions_record(&extrude.regions),
            region_references: region_references_record(&extrude.regions),
            extent,
            operation: operation_record(extrude.operation),
        })
    };
    match &extrude.extent {
        ExtrudeExtent::Symmetric { distance } => distances(ExtrudeExtentRecord::Symmetric {
            distance: distance.to_stored_text(),
        }),
        ExtrudeExtent::OneSide {
            end: ExtrudeEnd::Distance(distance),
            reversed,
        } => distances(ExtrudeExtentRecord::OneSide {
            distance: distance.to_stored_text(),
            reversed: *reversed,
        }),
        ExtrudeExtent::TwoSides {
            forward: ExtrudeEnd::Distance(forward),
            backward: ExtrudeEnd::Distance(backward),
        } => distances(ExtrudeExtentRecord::TwoSides {
            forward: forward.to_stored_text(),
            backward: backward.to_stored_text(),
        }),
        ExtrudeExtent::OneSide { end, reversed } => ends(ExtrudeEndsRecord::OneSide {
            end: end_record(end),
            reversed: *reversed,
        }),
        ExtrudeExtent::TwoSides { forward, backward } => ends(ExtrudeEndsRecord::TwoSides {
            forward: end_record(forward),
            backward: end_record(backward),
        }),
    }
}

fn revolve_record(revolve: &Revolve) -> FeatureKindRecord {
    let (sketch, regions, region_references, operation) = (
        revolve.sketch.raw(),
        regions_record(&revolve.regions),
        region_references_record(&revolve.regions),
        operation_record(revolve.operation),
    );
    let axis = match &revolve.axis {
        RevolveAxis::Sketch(line) => RevolveAxisRecord::Sketch(line.raw()),
        RevolveAxis::Model(axis) => RevolveAxisRecord::Model(Box::new(axis_record(axis))),
    };
    let extent = match &revolve.extent {
        RevolveExtent::Full => RevolveExtentRecord::Full,
        RevolveExtent::OneSide { angle, reversed } => RevolveExtentRecord::OneSide {
            angle: angle.to_stored_text(),
            reversed: *reversed,
        },
        RevolveExtent::Symmetric { angle } => RevolveExtentRecord::Symmetric {
            angle: angle.to_stored_text(),
        },
        RevolveExtent::TwoSides { forward, backward } => {
            return FeatureKindRecord::RevolveTwoAngles(RevolveTwoAnglesRecord {
                sketch,
                regions,
                region_references,
                axis,
                forward: forward.to_stored_text(),
                backward: backward.to_stored_text(),
                operation,
            });
        }
    };
    FeatureKindRecord::Revolve(RevolveRecord {
        sketch,
        regions,
        region_references,
        axis,
        extent,
        operation,
    })
}

fn pattern_record(pattern: &Pattern) -> FeatureKindRecord {
    let body = pattern.body.raw();
    match &pattern.kind {
        PatternKind::Linear { first, second } => {
            FeatureKindRecord::LinearPattern(Box::new(LinearPatternRecord {
                body,
                first: direction_record(first),
                second: second.as_ref().map(direction_record),
            }))
        }
        PatternKind::Circular(circular) => {
            FeatureKindRecord::CircularPattern(Box::new(CircularPatternRecord {
                body,
                axis: axis_record(&circular.axis),
                count: circular.count.to_stored_text(),
                angle: circular.angle.to_stored_text(),
                reversed: circular.reversed,
            }))
        }
    }
}

fn direction_record(direction: &LinearDirection) -> DirectionRecord {
    DirectionRecord {
        axis: axis_record(&direction.axis),
        count: direction.count.to_stored_text(),
        spacing: direction.spacing.to_stored_text(),
        reversed: direction.reversed,
    }
}

fn edge_record(edge: &EdgeReference) -> EdgeRecord {
    let [first, second] = edge.faces();
    let [from, to] = edge.ends();
    EdgeRecord {
        name: hex(edge.name().digest()),
        faces: [hex(first.digest()), hex(second.digest())],
        ends: [hex(from.digest()), hex(to.digest())],
        origins: edge
            .origins()
            .iter()
            .any(Option::is_some)
            .then(|| edge.origins().map(|origin| origin.map(origin_record))),
        copies: edge
            .origins()
            .iter()
            .flatten()
            .any(|origin| origin.copy().is_some())
            .then(|| {
                edge.origins()
                    .map(|origin| origin.as_ref().and_then(copy_record))
            }),
    }
}

fn regions_record(regions: &RegionChoice) -> RegionsRecord {
    match regions {
        RegionChoice::All => RegionsRecord::All,
        RegionChoice::Chosen(references) => RegionsRecord::Chosen(
            references
                .iter()
                .map(|reference| hex(reference.key().digest()))
                .collect(),
        ),
    }
}

fn region_references_record(regions: &RegionChoice) -> RegionReferencesRecord {
    let RegionChoice::Chosen(references) = regions else {
        return None;
    };
    let captured = |reference: &RegionReference| {
        !reference.boundary().is_empty() || reference.anchor().is_some()
    };
    references.iter().any(captured).then(|| {
        references
            .iter()
            .map(|reference| {
                Lenient::Read(RegionReferenceRecord {
                    boundary: reference
                        .boundary()
                        .iter()
                        .map(|piece| BoundaryPieceRecord {
                            entity: piece.entity,
                            side: match piece.side {
                                Side::Left => SideRecord::Left,
                                Side::Right => SideRecord::Right,
                            },
                            piece: hex(piece.piece),
                        })
                        .collect(),
                    anchor: reference.anchor().map(|anchor| anchor.to_array()),
                })
            })
            .collect()
    })
}

fn operation_record(operation: BodyOperation) -> OperationRecord {
    match operation {
        BodyOperation::NewBody => OperationRecord::NewBody,
        BodyOperation::Add(body) => OperationRecord::Add(body.raw()),
        BodyOperation::Remove(body) => OperationRecord::Remove(body.raw()),
        BodyOperation::Intersect(body) => OperationRecord::Intersect(body.raw()),
    }
}

fn plane_record(plane: Plane) -> PlaneRecord {
    PlaneRecord {
        origin: plane.origin().to_array(),
        normal: plane.normal().to_array(),
        x_axis: plane.x_axis().to_array(),
    }
}

fn hex(digest: u128) -> String {
    format!("{digest:032x}")
}

fn principal_plane_record(plane: PrincipalPlane) -> PrincipalPlaneRecord {
    match plane {
        PrincipalPlane::Xy => PrincipalPlaneRecord::Xy,
        PrincipalPlane::Xz => PrincipalPlaneRecord::Xz,
        PrincipalPlane::Yz => PrincipalPlaneRecord::Yz,
    }
}

fn principal_axis_record(axis: PrincipalAxis) -> PrincipalAxisRecord {
    match axis {
        PrincipalAxis::X => PrincipalAxisRecord::X,
        PrincipalAxis::Y => PrincipalAxisRecord::Y,
        PrincipalAxis::Z => PrincipalAxisRecord::Z,
    }
}

fn principal_geometry_record(geometry: PrincipalGeometry) -> PrincipalGeometryRecord {
    match geometry {
        PrincipalGeometry::Origin => PrincipalGeometryRecord::Origin,
        PrincipalGeometry::Axis(axis) => PrincipalGeometryRecord::Axis(principal_axis_record(axis)),
        PrincipalGeometry::Plane(plane) => {
            PrincipalGeometryRecord::Plane(principal_plane_record(plane))
        }
    }
}

pub(crate) fn principal_record(document: &Document) -> Option<PrincipalRecord> {
    let hidden: Vec<PrincipalGeometryRecord> = document
        .hidden_principal()
        .map(principal_geometry_record)
        .collect();
    (!hidden.is_empty()).then_some(PrincipalRecord { hidden })
}

pub(crate) fn suppressed_record(document: &Document) -> Option<SuppressedRecord> {
    let features: Vec<u64> = document
        .features()
        .filter(|feature| feature.suppressed)
        .map(|feature| feature.id().raw())
        .collect();
    (!features.is_empty()).then_some(SuppressedRecord { features })
}

pub(crate) fn rollback_record(document: &Document) -> Option<RollbackRecord> {
    match document.rollback_bar() {
        RollbackBar::AtEnd => None,
        RollbackBar::Before(feature) => Some(RollbackRecord {
            before: feature.raw(),
        }),
    }
}

fn restore_rollback(before: Option<u64>) -> RollbackBar {
    before.map_or(RollbackBar::AtEnd, |feature| {
        RollbackBar::Before(FeatureId::from_raw(feature))
    })
}

fn restore_principal_plane(record: PrincipalPlaneRecord) -> PrincipalPlane {
    match record {
        PrincipalPlaneRecord::Xy => PrincipalPlane::Xy,
        PrincipalPlaneRecord::Xz => PrincipalPlane::Xz,
        PrincipalPlaneRecord::Yz => PrincipalPlane::Yz,
    }
}

fn restore_principal_axis(record: PrincipalAxisRecord) -> PrincipalAxis {
    match record {
        PrincipalAxisRecord::X => PrincipalAxis::X,
        PrincipalAxisRecord::Y => PrincipalAxis::Y,
        PrincipalAxisRecord::Z => PrincipalAxis::Z,
    }
}

pub(crate) fn restore_principal(record: PrincipalGeometryRecord) -> PrincipalGeometry {
    match record {
        PrincipalGeometryRecord::Origin => PrincipalGeometry::Origin,
        PrincipalGeometryRecord::Axis(axis) => {
            PrincipalGeometry::Axis(restore_principal_axis(axis))
        }
        PrincipalGeometryRecord::Plane(plane) => {
            PrincipalGeometry::Plane(restore_principal_plane(plane))
        }
    }
}

fn plane_reference_record(reference: &PlaneReference) -> PlaneReferenceRecord {
    match reference {
        PlaneReference::Principal(plane) => {
            PlaneReferenceRecord::Principal(principal_plane_record(*plane))
        }
        PlaneReference::Datum(feature) => PlaneReferenceRecord::Datum(feature.raw()),
        PlaneReference::Face(attachment) => {
            PlaneReferenceRecord::Face(attachment_record(attachment))
        }
    }
}

fn axis_record(reference: &AxisReference) -> AxisReferenceRecord {
    match reference {
        AxisReference::Principal(axis) => {
            AxisReferenceRecord::Principal(principal_axis_record(*axis))
        }
        AxisReference::Datum(feature) => AxisReferenceRecord::Datum(feature.raw()),
        AxisReference::Edge { body, edge } => AxisReferenceRecord::Edge {
            body: body.raw(),
            edge: edge_record(edge),
        },
        AxisReference::Face { body, face } => AxisReferenceRecord::Face {
            body: body.raw(),
            face: face_record(face),
        },
    }
}

fn face_record(face: &FaceReference) -> FaceRecord {
    FaceRecord {
        face: hex(face.name().digest()),
        origin: face.origin().map(origin_record),
        neighbours: face
            .neighbours()
            .iter()
            .map(|name| hex(name.digest()))
            .collect(),
        copy: face.origin().and_then(|origin| copy_record(&origin)),
    }
}

fn copy_record(origin: &FaceOrigin) -> Option<CopyRecord> {
    origin.copy().map(|copy| CopyRecord {
        pattern: copy.pattern,
        index: copy.index,
    })
}

fn origin_record(origin: FaceOrigin) -> FaceOriginRecord {
    match origin {
        FaceOrigin::Side { feature, entity } => FaceOriginRecord::Side { feature, entity },
        FaceOrigin::StartCap { feature } => FaceOriginRecord::StartCap { feature },
        FaceOrigin::EndCap { feature } => FaceOriginRecord::EndCap { feature },
        FaceOrigin::Fillet { feature } => FaceOriginRecord::Fillet { feature },
        FaceOrigin::Chamfer { feature } => FaceOriginRecord::Chamfer { feature },
        FaceOrigin::Shell { feature } => FaceOriginRecord::Shell { feature },
        FaceOrigin::Imported { feature, face } => FaceOriginRecord::Imported { feature, face },
        FaceOrigin::Copy { .. } => origin_record(origin.original()),
    }
}

fn attachment_record(attachment: &FaceAttachment) -> AttachmentRecord {
    let FaceRecord {
        face,
        origin,
        neighbours,
        copy,
    } = face_record(&attachment.face);
    AttachmentRecord {
        body: attachment.body.raw(),
        face,
        origin,
        neighbours,
        copy,
    }
}

fn sketch_record(feature: &SketchFeature) -> SketchRecord {
    let sketch = &feature.sketch;
    SketchRecord {
        plane: plane_record(sketch.plane()),
        attachment: feature
            .attachment
            .as_ref()
            .and_then(SketchAttachment::face)
            .map(|attachment| Lenient::Read(attachment_record(attachment))),
        datum: feature
            .attachment
            .as_ref()
            .and_then(SketchAttachment::datum)
            .map(FeatureId::raw),
        entities: sketch
            .entities()
            .map(|(id, entity)| {
                Lenient::Read(entity_record(id, entity, sketch.is_construction(id)))
            })
            .collect(),
        constraints: sketch
            .constraints()
            .map(|(id, constraint)| Lenient::Read(constraint_record(id, constraint)))
            .collect(),
        next_id: sketch.next_id(),
    }
}

fn entity_record(id: EntityId, entity: &Entity, construction: bool) -> EntityRecord {
    EntityRecord {
        id: id.raw(),
        construction,
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
        Constraint::Angle {
            from,
            to,
            reversed,
            value,
        } => ConstraintKindRecord::Angle {
            from: from.raw(),
            to: to.raw(),
            reversed: *reversed,
            value: value.to_stored_text(),
        },
        Constraint::Radius { entity, value } => ConstraintKindRecord::Radius {
            entity: entity.raw(),
            value: value.to_stored_text(),
        },
        Constraint::HorizontalPoints(a, b) => ConstraintKindRecord::HorizontalPoints(pair(a, b)),
        Constraint::VerticalPoints(a, b) => ConstraintKindRecord::VerticalPoints(pair(a, b)),
        Constraint::Midpoint { point, line } => ConstraintKindRecord::Midpoint {
            point: point.raw(),
            line: line.raw(),
        },
        Constraint::Concentric(a, b) => ConstraintKindRecord::Concentric(pair(a, b)),
        Constraint::Collinear(a, b) => ConstraintKindRecord::Collinear(pair(a, b)),
        Constraint::Symmetric {
            first,
            second,
            about,
        } => ConstraintKindRecord::Symmetric {
            first: first.raw(),
            second: second.raw(),
            about: about.raw(),
        },
        Constraint::Fix { point, at } => ConstraintKindRecord::Fix {
            point: point.raw(),
            at: at.to_array(),
        },
        Constraint::HorizontalDistance { from, to, value } => {
            ConstraintKindRecord::HorizontalDistance {
                from: from.raw(),
                to: to.raw(),
                value: value.to_stored_text(),
            }
        }
        Constraint::VerticalDistance { from, to, value } => {
            ConstraintKindRecord::VerticalDistance {
                from: from.raw(),
                to: to.raw(),
                value: value.to_stored_text(),
            }
        }
        Constraint::Diameter { entity, value } => ConstraintKindRecord::Diameter {
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
            suppressed: feature.suppressed,
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
        Edit::SetFeatureHidden { id, hidden } => EditRecord::SetFeatureHidden {
            id: id.raw(),
            hidden: *hidden,
        },
        Edit::SetFeatureSuppressed { id, suppressed } => EditRecord::SetFeatureSuppressed {
            id: id.raw(),
            suppressed: *suppressed,
        },
        Edit::SetRollbackBar { bar } => EditRecord::SetRollbackBar {
            before: match bar {
                RollbackBar::AtEnd => None,
                RollbackBar::Before(feature) => Some(feature.raw()),
            },
        },
        Edit::SetPrincipalHidden { geometry, hidden } => EditRecord::SetPrincipalHidden {
            geometry: principal_geometry_record(*geometry),
            hidden: *hidden,
        },
        Edit::SetFeatureKind { id, kind } => EditRecord::SetFeatureKind {
            feature: FeatureRecord {
                id: id.raw(),
                name: String::new(),
                hidden: false,
                kind: feature_kind_record(kind),
            },
        },
        Edit::SetSketchPlacement {
            feature,
            plane,
            attachment,
        } => EditRecord::SetSketchPlacement {
            feature: feature.raw(),
            plane: plane_record(*plane),
            attachment: attachment
                .as_ref()
                .and_then(SketchAttachment::face)
                .map(attachment_record),
            datum: attachment
                .as_ref()
                .and_then(SketchAttachment::datum)
                .map(FeatureId::raw),
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
            construction,
        } => EditRecord::AddSketchEntity {
            feature: feature.raw(),
            entity: entity_record(*id, entity, *construction),
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
            entity: entity_record(*id, entity, false),
        },
        Edit::SetSketchConstruction {
            feature,
            id,
            construction,
        } => EditRecord::SetSketchConstruction {
            feature: feature.raw(),
            id: id.raw(),
            construction: *construction,
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
        EditRecord::InsertFeature {
            index,
            feature,
            suppressed,
        } => {
            let mut issues = Vec::new();
            let mut feature = restore_feature(&feature, &mut issues);
            if !issues.is_empty() {
                return None;
            }
            feature.suppressed = suppressed;
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
        EditRecord::SetFeatureHidden { id, hidden } => Edit::SetFeatureHidden {
            id: FeatureId::from_raw(id),
            hidden,
        },
        EditRecord::SetFeatureSuppressed { id, suppressed } => Edit::SetFeatureSuppressed {
            id: FeatureId::from_raw(id),
            suppressed,
        },
        EditRecord::SetRollbackBar { before } => Edit::SetRollbackBar {
            bar: restore_rollback(before),
        },
        EditRecord::SetPrincipalHidden { geometry, hidden } => Edit::SetPrincipalHidden {
            geometry: restore_principal(geometry),
            hidden,
        },
        EditRecord::SetFeatureKind { feature } => {
            let mut issues = Vec::new();
            let kind = restore_kind(&feature.kind, "", &mut issues);
            if !issues.is_empty() {
                return None;
            }
            Edit::SetFeatureKind {
                id: FeatureId::from_raw(feature.id),
                kind,
            }
        }
        EditRecord::SetSketchPlacement {
            feature,
            plane,
            attachment,
            datum,
        } => Edit::SetSketchPlacement {
            feature: FeatureId::from_raw(feature),
            plane: restore_plane(plane)?,
            attachment: match (attachment, datum) {
                (Some(record), _) => Some(SketchAttachment::Face(restore_attachment(&record)?)),
                (None, Some(datum)) => Some(SketchAttachment::Datum(FeatureId::from_raw(datum))),
                (None, None) => None,
            },
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
            construction: entity.construction,
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
        EditRecord::SetSketchConstruction {
            feature,
            id,
            construction,
        } => Edit::SetSketchConstruction {
            feature: FeatureId::from_raw(feature),
            id: EntityId::from_raw(id),
            construction,
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
    let kind = restore_kind(&record.kind, &name, issues);
    let mut feature = Feature::new(FeatureId::from_raw(record.id), name, kind);
    feature.hidden = record.hidden;
    feature
}

fn restore_kind(record: &FeatureKindRecord, name: &str, issues: &mut Vec<String>) -> FeatureKind {
    match record {
        FeatureKindRecord::Sketch(sketch) => {
            let attachment = match &sketch.attachment {
                None => None,
                Some(Lenient::Read(record)) => restore_attachment(record),
                Some(Lenient::Unreadable(_)) => None,
            };
            if sketch.attachment.is_some() && attachment.is_none() {
                issues.push(format!(
                    "The face that “{name}” lies on could not be read, so the sketch stays where \
                     it was."
                ));
            }
            let attachment = attachment.map(SketchAttachment::Face).or_else(|| {
                sketch
                    .datum
                    .map(|datum| SketchAttachment::Datum(FeatureId::from_raw(datum)))
            });
            FeatureKind::Sketch(SketchFeature {
                sketch: restore_sketch(sketch, name, issues),
                attachment,
            })
        }
        FeatureKindRecord::Extrude(extrude) => {
            let mut value =
                |text: &str, what: &str| restore_value(text, what, "10 mm", name, issues);
            let extent = match &extrude.extent {
                ExtrudeExtentRecord::OneSide { distance, reversed } => {
                    ExtrudeExtent::one_side(value(distance, "distance"), *reversed)
                }
                ExtrudeExtentRecord::Symmetric { distance } => ExtrudeExtent::Symmetric {
                    distance: value(distance, "distance"),
                },
                ExtrudeExtentRecord::TwoSides { forward, backward } => ExtrudeExtent::two_sides(
                    value(forward, "forward distance"),
                    value(backward, "backward distance"),
                ),
            };
            FeatureKind::Solid(SolidFeature::Extrude(Extrude {
                sketch: FeatureId::from_raw(extrude.sketch),
                regions: restore_regions(
                    &extrude.regions,
                    &extrude.region_references,
                    name,
                    issues,
                ),
                extent,
                operation: restore_operation(extrude.operation),
            }))
        }
        FeatureKindRecord::ExtrudeTo(extrude) => {
            let extent = match &extrude.extent {
                ExtrudeEndsRecord::OneSide { end, reversed } => ExtrudeExtent::OneSide {
                    end: restore_end(end, ("end", "distance"), name, issues),
                    reversed: *reversed,
                },
                ExtrudeEndsRecord::TwoSides { forward, backward } => ExtrudeExtent::TwoSides {
                    forward: restore_end(
                        forward,
                        ("forward end", "forward distance"),
                        name,
                        issues,
                    ),
                    backward: restore_end(
                        backward,
                        ("backward end", "backward distance"),
                        name,
                        issues,
                    ),
                },
            };
            FeatureKind::Solid(SolidFeature::Extrude(Extrude {
                sketch: FeatureId::from_raw(extrude.sketch),
                regions: restore_regions(
                    &extrude.regions,
                    &extrude.region_references,
                    name,
                    issues,
                ),
                extent,
                operation: restore_operation(extrude.operation),
            }))
        }
        FeatureKindRecord::Revolve(revolve) => {
            let mut value = |text: &str| restore_value(text, "angle", "360 deg", name, issues);
            let extent = match &revolve.extent {
                RevolveExtentRecord::Full => RevolveExtent::Full,
                RevolveExtentRecord::OneSide { angle, reversed } => RevolveExtent::OneSide {
                    angle: value(angle),
                    reversed: *reversed,
                },
                RevolveExtentRecord::Symmetric { angle } => RevolveExtent::Symmetric {
                    angle: value(angle),
                },
            };
            FeatureKind::Solid(SolidFeature::Revolve(Revolve {
                sketch: FeatureId::from_raw(revolve.sketch),
                regions: restore_regions(
                    &revolve.regions,
                    &revolve.region_references,
                    name,
                    issues,
                ),
                axis: restore_revolve_axis(&revolve.axis, name, issues),
                extent,
                operation: restore_operation(revolve.operation),
            }))
        }
        FeatureKindRecord::RevolveTwoAngles(revolve) => {
            let mut value =
                |text: &str, what: &str| restore_value(text, what, "180 deg", name, issues);
            let extent = RevolveExtent::TwoSides {
                forward: value(&revolve.forward, "forward angle"),
                backward: value(&revolve.backward, "backward angle"),
            };
            FeatureKind::Solid(SolidFeature::Revolve(Revolve {
                sketch: FeatureId::from_raw(revolve.sketch),
                regions: restore_regions(
                    &revolve.regions,
                    &revolve.region_references,
                    name,
                    issues,
                ),
                axis: restore_revolve_axis(&revolve.axis, name, issues),
                extent,
                operation: restore_operation(revolve.operation),
            }))
        }
        FeatureKindRecord::Fillet(record) => {
            FeatureKind::Blend(restore_blend(record, BlendKind::Fillet, name, issues))
        }
        FeatureKindRecord::Chamfer(record) => {
            FeatureKind::Blend(restore_blend(record, BlendKind::Chamfer, name, issues))
        }
        FeatureKindRecord::Shell(record) => FeatureKind::Shell(restore_shell(record, name, issues)),
        FeatureKindRecord::LinearPattern(record) => {
            FeatureKind::from(restore_linear_pattern(record, name, issues))
        }
        FeatureKindRecord::CircularPattern(record) => {
            FeatureKind::from(restore_circular_pattern(record, name, issues))
        }
        FeatureKindRecord::Plane(record) => {
            FeatureKind::Datum(Datum::Plane(restore_datum_plane(record, name, issues)))
        }
        FeatureKindRecord::Axis(record) => {
            FeatureKind::Datum(Datum::Axis(restore_datum_axis(record, name, issues)))
        }
        FeatureKindRecord::Import(record) => {
            FeatureKind::Import(restore_import(record, name, issues))
        }
    }
}

fn restore_revolve_axis(
    record: &RevolveAxisRecord,
    name: &str,
    issues: &mut Vec<String>,
) -> RevolveAxis {
    match record {
        RevolveAxisRecord::Sketch(line) => RevolveAxis::Sketch(EntityId::from_raw(*line)),
        RevolveAxisRecord::Model(axis) => match restore_axis(axis) {
            Some(axis) => RevolveAxis::Model(axis),
            None => {
                issues.push(format!(
                    "The axis of “{name}” could not be read, so it turns about the vertical axis \
                     of its sketch."
                ));
                RevolveAxis::Sketch(EntityId::VERTICAL_AXIS)
            }
        },
    }
}

fn restore_end(
    record: &Lenient<ExtrudeEndRecord>,
    (what, distance): (&str, &str),
    name: &str,
    issues: &mut Vec<String>,
) -> ExtrudeEnd {
    let fallback = || ExtrudeEnd::Distance(Expression::Measure(10.0, Unit::Millimetre));
    match record {
        Lenient::Read(ExtrudeEndRecord::Distance(text)) => {
            ExtrudeEnd::Distance(restore_value(text, distance, "10 mm", name, issues))
        }
        Lenient::Read(ExtrudeEndRecord::ThroughAll) => ExtrudeEnd::ThroughAll,
        Lenient::Read(ExtrudeEndRecord::UpToNext) => ExtrudeEnd::UpToNext,
        Lenient::Read(ExtrudeEndRecord::UpToFace(target)) => {
            match restore_plane_reference(target) {
                Some(target) => ExtrudeEnd::UpToFace(target),
                None => {
                    issues.push(format!(
                    "The face or plane that the {what} of “{name}” runs up to could not be read, \
                     so that end was set to 10 mm."
                ));
                    fallback()
                }
            }
        }
        Lenient::Unreadable(_) => {
            issues.push(format!(
                "The {what} of “{name}” could not be read, so it was set to 10 mm."
            ));
            fallback()
        }
    }
}

fn restore_import(record: &ImportRecord, name: &str, issues: &mut Vec<String>) -> Import {
    let solid = match caditor_step::read_step(&record.step) {
        Ok(mut model) if !model.solids.is_empty() => model.solids.swap_remove(0).solid,
        Ok(_) => Solid::default(),
        Err(error) => {
            issues.push(format!(
                "The shape of “{name}”, imported from “{}”, could not be read ({error}), so the \
                 feature has no shape.",
                record.source
            ));
            Solid::default()
        }
    };
    Import::new(record.source.clone(), solid, record.step.as_str())
}

fn restore_plane_reference(record: &PlaneReferenceRecord) -> Option<PlaneReference> {
    Some(match record {
        PlaneReferenceRecord::Principal(plane) => {
            PlaneReference::Principal(restore_principal_plane(*plane))
        }
        PlaneReferenceRecord::Datum(feature) => {
            PlaneReference::Datum(FeatureId::from_raw(*feature))
        }
        PlaneReferenceRecord::Face(attachment) => {
            PlaneReference::Face(restore_attachment(attachment)?)
        }
    })
}

fn restore_axis(record: &AxisReferenceRecord) -> Option<AxisReference> {
    Some(match record {
        AxisReferenceRecord::Principal(axis) => {
            AxisReference::Principal(restore_principal_axis(*axis))
        }
        AxisReferenceRecord::Datum(feature) => AxisReference::Datum(FeatureId::from_raw(*feature)),
        AxisReferenceRecord::Edge { body, edge } => AxisReference::Edge {
            body: FeatureId::from_raw(*body),
            edge: Box::new(restore_edge(edge)?),
        },
        AxisReferenceRecord::Face { body, face } => AxisReference::Face {
            body: FeatureId::from_raw(*body),
            face: restore_face(&face.face, face.origin, face.copy, &face.neighbours)?,
        },
    })
}

fn restore_datum_plane(
    record: &DatumPlaneRecord,
    feature: &str,
    issues: &mut Vec<String>,
) -> DatumPlane {
    let base = restore_plane_reference(&record.base).unwrap_or_else(|| {
        issues.push(format!(
            "What “{feature}” was based on could not be read, so it is based on the XY plane."
        ));
        PlaneReference::Principal(PrincipalPlane::Xy)
    });
    let rotation = record.rotation.as_ref().and_then(|rotation| {
        let Some(axis) = restore_axis(&rotation.axis) else {
            issues.push(format!(
                "The axis “{feature}” turns about could not be read, so it no longer turns."
            ));
            return None;
        };
        Some(PlaneRotation {
            axis,
            angle: restore_value(&rotation.angle, "angle", "0 deg", feature, issues),
        })
    });
    DatumPlane {
        base,
        rotation,
        offset: restore_value(&record.offset, "offset", "0 mm", feature, issues),
    }
}

fn restore_datum_axis(
    record: &DatumAxisRecord,
    feature: &str,
    issues: &mut Vec<String>,
) -> DatumAxis {
    let restored = match record {
        DatumAxisRecord::Along(axis) => restore_axis(axis).map(DatumAxis::Along),
        DatumAxisRecord::Intersection([first, second]) => restore_plane_reference(first)
            .zip(restore_plane_reference(second))
            .map(|(first, second)| DatumAxis::Intersection(first, second)),
    };
    restored.unwrap_or_else(|| {
        issues.push(format!(
            "What “{feature}” runs along could not be read, so it runs along the Z axis."
        ));
        DatumAxis::Along(AxisReference::Principal(PrincipalAxis::Z))
    })
}

fn restore_direction(
    record: &DirectionRecord,
    which: &str,
    feature: &str,
    issues: &mut Vec<String>,
) -> Option<LinearDirection> {
    let axis = restore_axis(&record.axis)?;
    Some(LinearDirection {
        axis,
        count: restore_value(
            &record.count,
            &format!("{which}count"),
            "1",
            feature,
            issues,
        ),
        spacing: restore_value(
            &record.spacing,
            &format!("{which}spacing"),
            "10 mm",
            feature,
            issues,
        ),
        reversed: record.reversed,
    })
}

fn restore_linear_pattern(
    record: &LinearPatternRecord,
    feature: &str,
    issues: &mut Vec<String>,
) -> Pattern {
    let first = restore_direction(&record.first, "", feature, issues).unwrap_or_else(|| {
        issues.push(format!(
            "The direction of “{feature}” could not be read, so it runs along the X axis."
        ));
        LinearDirection {
            axis: AxisReference::Principal(PrincipalAxis::X),
            count: restore_value(&record.first.count, "count", "1", feature, issues),
            spacing: restore_value(&record.first.spacing, "spacing", "10 mm", feature, issues),
            reversed: record.first.reversed,
        }
    });
    let second = record.second.as_ref().and_then(|second| {
        let restored = restore_direction(second, "second ", feature, issues);
        if restored.is_none() {
            issues.push(format!(
                "The second direction of “{feature}” could not be read, so it was left out."
            ));
        }
        restored
    });
    Pattern {
        body: FeatureId::from_raw(record.body),
        kind: PatternKind::Linear { first, second },
    }
}

fn restore_circular_pattern(
    record: &CircularPatternRecord,
    feature: &str,
    issues: &mut Vec<String>,
) -> Pattern {
    let axis = restore_axis(&record.axis).unwrap_or_else(|| {
        issues.push(format!(
            "The axis of “{feature}” could not be read, so it turns about the Z axis."
        ));
        AxisReference::Principal(PrincipalAxis::Z)
    });
    Pattern {
        body: FeatureId::from_raw(record.body),
        kind: PatternKind::Circular(CircularPattern {
            axis,
            count: restore_value(&record.count, "count", "1", feature, issues),
            angle: restore_value(&record.angle, "angle", "360 deg", feature, issues),
            reversed: record.reversed,
        }),
    }
}

fn restore_shell(record: &ShellRecord, feature: &str, issues: &mut Vec<String>) -> Shell {
    let thickness = restore_value(&record.thickness, "thickness", "1 mm", feature, issues);
    let open: Vec<FaceReference> = record
        .open
        .iter()
        .filter_map(|face| match face {
            Lenient::Read(face) => {
                restore_face(&face.face, face.origin, face.copy, &face.neighbours)
            }
            Lenient::Unreadable(_) => None,
        })
        .collect();
    if open.len() < record.open.len() {
        issues.push(format!(
            "Some faces opened by “{feature}” could not be read and were left closed."
        ));
    }
    Shell {
        body: FeatureId::from_raw(record.body),
        open,
        thickness,
    }
}

fn restore_blend(
    record: &BlendRecord,
    kind: BlendKind,
    feature: &str,
    issues: &mut Vec<String>,
) -> Blend {
    let size = restore_value(&record.size, kind.size_name(), "1 mm", feature, issues);
    let edges: Vec<EdgeReference> = record
        .edges
        .iter()
        .filter_map(|edge| match edge {
            Lenient::Read(edge) => restore_edge(edge),
            Lenient::Unreadable(_) => None,
        })
        .collect();
    if edges.len() < record.edges.len() {
        issues.push(format!(
            "Some edges chosen for “{feature}” could not be read and were left out."
        ));
    }
    Blend {
        kind,
        body: FeatureId::from_raw(record.body),
        edges,
        size,
    }
}

fn restore_edge(record: &EdgeRecord) -> Option<EdgeReference> {
    let face = |text: &str| restore_digest(text).map(FaceName::from_digest);
    let vertex = |text: &str| restore_digest(text).map(VertexName::from_digest);
    let [first, second] = &record.faces;
    let [from, to] = &record.ends;
    let [first_copy, second_copy] = record.copies.unwrap_or_default();
    let [first_origin, second_origin] = record.origins.unwrap_or_default();
    let first_origin = first_origin.map(|origin| restore_copied_origin(origin, first_copy));
    let second_origin = second_origin.map(|origin| restore_copied_origin(origin, second_copy));
    let mut sides = [(face(first)?, first_origin), (face(second)?, second_origin)];
    sides.sort_by_key(|(name, _)| *name);
    Some(
        EdgeReference::new(
            EdgeName::from_digest(restore_digest(&record.name)?),
            sides.map(|(name, _)| name),
            [vertex(from)?, vertex(to)?],
        )
        .with_origins(sides.map(|(_, origin)| origin)),
    )
}

fn restore_value(
    text: &str,
    what: &str,
    fallback: &str,
    feature: &str,
    issues: &mut Vec<String>,
) -> Expression {
    if let Ok(value) = Expression::parse_stored(text) {
        return value;
    }
    issues.push(format!(
        "The {what} of “{feature}” could not be read, so it was set to {fallback}."
    ));
    Expression::parse_stored(fallback).unwrap_or(Expression::Number(10.0))
}

fn restore_region_reference(
    key: RegionKey,
    record: Option<&RegionReferenceRecord>,
) -> RegionReference {
    let Some(record) = record else {
        return RegionReference::of_key(key);
    };
    let boundary = record
        .boundary
        .iter()
        .filter_map(|piece| {
            Some(BoundaryPiece {
                entity: piece.entity,
                side: match piece.side {
                    SideRecord::Left => Side::Left,
                    SideRecord::Right => Side::Right,
                },
                piece: restore_digest(&piece.piece)?,
            })
        })
        .collect();
    let anchor = record
        .anchor
        .map(Point2::from_array)
        .filter(|anchor| anchor.is_finite());
    RegionReference::new(key, boundary, anchor)
}

fn restore_regions(
    record: &RegionsRecord,
    references: &RegionReferencesRecord,
    feature: &str,
    issues: &mut Vec<String>,
) -> RegionChoice {
    let RegionsRecord::Chosen(keys) = record else {
        return RegionChoice::All;
    };
    let reference_at = |index: usize| match references.as_ref()?.get(index)? {
        Lenient::Read(record) => Some(record),
        Lenient::Unreadable(_) => None,
    };
    let read: Vec<RegionReference> = keys
        .iter()
        .enumerate()
        .filter_map(|(index, key)| {
            let key = RegionKey::from_digest(restore_digest(key)?);
            Some(restore_region_reference(key, reference_at(index)))
        })
        .collect();
    if read.len() == keys.len() {
        return RegionChoice::Chosen(read);
    }
    if read.is_empty() {
        issues.push(format!(
            "The chosen regions of “{feature}” could not be read, so all regions are used."
        ));
        return RegionChoice::All;
    }
    issues.push(format!(
        "Some chosen regions of “{feature}” could not be read and were left out."
    ));
    RegionChoice::Chosen(read)
}

fn restore_operation(record: OperationRecord) -> BodyOperation {
    match record {
        OperationRecord::NewBody => BodyOperation::NewBody,
        OperationRecord::Add(body) => BodyOperation::Add(FeatureId::from_raw(body)),
        OperationRecord::Remove(body) => BodyOperation::Remove(FeatureId::from_raw(body)),
        OperationRecord::Intersect(body) => BodyOperation::Intersect(FeatureId::from_raw(body)),
    }
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
        let id = EntityId::from_raw(record.id);
        if let Err(error) = sketch.insert_entity(id, entity) {
            issues.push(format!(
                "In “{feature}”, {label} was left out because {error}."
            ));
        } else if record.construction
            && let Err(error) = sketch.set_construction(id, true)
        {
            issues.push(format!(
                "In “{feature}”, {label} was kept as ordinary geometry because {error}."
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

fn restore_digest(text: &str) -> Option<u128> {
    u128::from_str_radix(text, 16).ok()
}

fn restore_attachment(record: &AttachmentRecord) -> Option<FaceAttachment> {
    Some(FaceAttachment {
        body: FeatureId::from_raw(record.body),
        face: restore_face(&record.face, record.origin, record.copy, &record.neighbours)?,
    })
}

fn restore_face(
    face: &str,
    origin: Option<FaceOriginRecord>,
    copy: Option<CopyRecord>,
    neighbours: &[String],
) -> Option<FaceReference> {
    let neighbours = neighbours
        .iter()
        .map(|text| restore_digest(text).map(FaceName::from_digest))
        .collect::<Option<Vec<FaceName>>>()?;
    Some(FaceReference::new(
        FaceName::from_digest(restore_digest(face)?),
        origin.map(|origin| restore_copied_origin(origin, copy)),
        neighbours,
    ))
}

fn restore_copied_origin(origin: FaceOriginRecord, copy: Option<CopyRecord>) -> FaceOrigin {
    restore_origin(origin).with_copy(copy.map(|copy| FaceCopy {
        pattern: copy.pattern,
        index: copy.index,
    }))
}

fn restore_origin(origin: FaceOriginRecord) -> FaceOrigin {
    match origin {
        FaceOriginRecord::Side { feature, entity } => FaceOrigin::Side { feature, entity },
        FaceOriginRecord::StartCap { feature } => FaceOrigin::StartCap { feature },
        FaceOriginRecord::EndCap { feature } => FaceOrigin::EndCap { feature },
        FaceOriginRecord::Fillet { feature } => FaceOrigin::Fillet { feature },
        FaceOriginRecord::Chamfer { feature } => FaceOrigin::Chamfer { feature },
        FaceOriginRecord::Shell { feature } => FaceOrigin::Shell { feature },
        FaceOriginRecord::Imported { feature, face } => FaceOrigin::Imported { feature, face },
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
            reversed,
            value: text,
        } => {
            let (from, to, reversed) = (entity(*from), entity(*to), *reversed);
            let value = value(text, DrawnValue::Angle { from, to, reversed })?;
            Constraint::Angle {
                from,
                to,
                reversed,
                value,
            }
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
        ConstraintKindRecord::HorizontalPoints(ids) => {
            let (a, b) = pair(*ids);
            Constraint::HorizontalPoints(a, b)
        }
        ConstraintKindRecord::VerticalPoints(ids) => {
            let (a, b) = pair(*ids);
            Constraint::VerticalPoints(a, b)
        }
        ConstraintKindRecord::Midpoint { point, line } => Constraint::Midpoint {
            point: entity(*point),
            line: entity(*line),
        },
        ConstraintKindRecord::Concentric(ids) => {
            let (a, b) = pair(*ids);
            Constraint::Concentric(a, b)
        }
        ConstraintKindRecord::Collinear(ids) => {
            let (a, b) = pair(*ids);
            Constraint::Collinear(a, b)
        }
        ConstraintKindRecord::Symmetric {
            first,
            second,
            about,
        } => Constraint::Symmetric {
            first: entity(*first),
            second: entity(*second),
            about: entity(*about),
        },
        ConstraintKindRecord::Fix { point, at: [x, y] } => Constraint::Fix {
            point: entity(*point),
            at: Point2::new(*x, *y),
        },
        ConstraintKindRecord::HorizontalDistance {
            from,
            to,
            value: text,
        } => {
            let (from, to) = (entity(*from), entity(*to));
            let value = value(text, DrawnValue::HorizontalDistance { from, to })?;
            Constraint::HorizontalDistance { from, to, value }
        }
        ConstraintKindRecord::VerticalDistance {
            from,
            to,
            value: text,
        } => {
            let (from, to) = (entity(*from), entity(*to));
            let value = value(text, DrawnValue::VerticalDistance { from, to })?;
            Constraint::VerticalDistance { from, to, value }
        }
        ConstraintKindRecord::Diameter {
            entity: curve,
            value: text,
        } => {
            let curve = entity(*curve);
            let value = value(text, DrawnValue::Diameter(curve))?;
            Constraint::Diameter {
                entity: curve,
                value,
            }
        }
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DrawnValue {
    Distance {
        from: EntityId,
        to: EntityId,
    },
    HorizontalDistance {
        from: EntityId,
        to: EntityId,
    },
    VerticalDistance {
        from: EntityId,
        to: EntityId,
    },
    Angle {
        from: EntityId,
        to: EntityId,
        reversed: bool,
    },
    Radius(EntityId),
    Diameter(EntityId),
}

impl DrawnValue {
    fn noun(self) -> &'static str {
        match self {
            Self::Distance { .. } => "a distance",
            Self::HorizontalDistance { .. } => "a horizontal distance",
            Self::VerticalDistance { .. } => "a vertical distance",
            Self::Angle { .. } => "an angle",
            Self::Radius(_) => "a radius",
            Self::Diameter(_) => "a diameter",
        }
    }

    fn drawn_name(self) -> &'static str {
        match self {
            Self::Distance { .. }
            | Self::HorizontalDistance { .. }
            | Self::VerticalDistance { .. } => "drawn length",
            Self::Angle { .. } => "drawn angle",
            Self::Radius(_) => "drawn radius",
            Self::Diameter(_) => "drawn diameter",
        }
    }

    fn measure(self, sketch: &Sketch) -> Option<Quantity> {
        let value = Expression::Number(0.0);
        let constraint = match self {
            Self::Distance { from, to } => Constraint::Distance { from, to, value },
            Self::HorizontalDistance { from, to } => {
                Constraint::HorizontalDistance { from, to, value }
            }
            Self::VerticalDistance { from, to } => Constraint::VerticalDistance { from, to, value },
            Self::Angle { from, to, reversed } => Constraint::Angle {
                from,
                to,
                reversed,
                value,
            },
            Self::Radius(entity) => Constraint::Radius { entity, value },
            Self::Diameter(entity) => Constraint::Diameter { entity, value },
        };
        let measured = sketch.measured(&constraint)?;
        let quantity = match self {
            Self::Angle { .. } => Quantity::angle(measured),
            Self::Radius(_) | Self::Diameter(_) if measured <= 0.0 => return None,
            Self::Distance { .. }
            | Self::HorizontalDistance { .. }
            | Self::VerticalDistance { .. }
            | Self::Radius(_)
            | Self::Diameter(_) => Quantity::length(measured),
        };
        Some(quantity)
    }
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
