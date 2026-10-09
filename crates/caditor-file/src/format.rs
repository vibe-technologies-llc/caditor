use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
};

use caditor_document::{
    AxisMate, AxisReference, AxisSide, AxisTurn, Blend, BlendKind, BodyAppearance, BodyOperation,
    BodyPlacement, CircularPattern, Combine, CombineOperation, CurveStation, Datum, DatumAxis,
    DatumFrame, DatumPlane, DatumPoint, Document, Edit, Extrude, ExtrudeEnd, ExtrudeExtent,
    FaceAttachment, FaceColour, FaceMate, FaceTangent, Feature, FeatureId, FeatureKind,
    HOME_VIEW_NAME, Hole, HoleBottom, HoleDepth, HoleFit, HoleShape, HoleSizing, HoleStandard,
    HoleStep, HoleStyle, Import, LinearDirection, LinearSpacing, MAX_BODY_NAME_CHARS,
    MAX_GROUP_NAME_CHARS, MAX_MATERIAL_NAME_CHARS, MAX_PATTERN_INSTANCES, MAX_SAVED_VIEWS,
    MAX_VIEW_NAME_CHARS, MIN_OPACITY_PERCENT, Mate, MatePair, MetricSize, Mirror, ModelProperties,
    ModelProperty, Move, NamedView, OPAQUE_PERCENT, ORIGINAL_INSTANCE, OffsetFace, Parameter,
    ParameterOwner, Pattern, PatternKind, PlaneReference, PlaneRotation, PlaneThrough, PointBy,
    PointReference, Primitive, PrimitiveAnchor, PrimitiveShape, PrincipalAxis, PrincipalGeometry,
    PrincipalPlane, ProjectionSource, RegionChoice, Remove, Revolve, RevolveAxis, RevolveExtent,
    Rgb, RollbackBar, SavedView, SavedViews, Scale, Shell, SketchAttachment, SketchFeature,
    SolidFeature, SolidStart, Split, SplitAlong, TappedThread, Thread, ThreadFamily, ThreadHand,
    ThreadLength, ThreadSide, ThreadSize, Transaction, TurnCentre, group_name, material_name,
    view_name,
};
use caditor_expression::{BinaryOperator, Expression, ParameterId, Quantity, Unit};
use caditor_geometry::{Plane, Point2, Point3, Rotation3, Vector2, Vector3};
use caditor_kernel::{
    BoundaryPiece, EdgeName, EdgeReference, FaceCopy, FaceName, FaceOrigin, FaceReference,
    RegionKey, RegionReference, Side, Solid, VertexName,
};
use caditor_sketch::{Constraint, ConstraintId, Entity, EntityId, Sketch};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::DeserializeOwned};
use serde_json::Value;

use crate::selection_sets::{
    SelectionSetsRecord, restore_selection_sets, selection_sets_record_of,
};

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
    Properties(PropertiesRecord),
    Views(ViewsRecord),
    NamedValues(NamedValuesRecord),
    SelectionSets(SelectionSetsRecord),
}

pub(crate) const RECORD_KINDS: [&str; 10] = [
    "parameter",
    "feature",
    "next_ids",
    "principal",
    "suppressed",
    "rollback",
    "properties",
    "views",
    "named_values",
    "selection_sets",
];

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct NamedValuesRecord {
    pub values: Vec<Lenient<NamedValueRecord>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct NamedValueRecord {
    pub parameter: u64,
    pub owner: OwnerRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OwnerRecord {
    Feature { feature: u64, value: String },
    Dimension { sketch: u64, constraint: u64 },
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct PropertiesRecord {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub part_number: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub revision: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub author: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub organisation: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct ViewsRecord {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub named: Vec<Lenient<NamedViewRecord>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home: Option<Lenient<ViewRecord>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct NamedViewRecord {
    pub name: String,
    pub view: ViewRecord,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(crate) struct ViewRecord {
    pub target: [f64; 3],
    pub orientation: [f64; 4],
    pub distance: f64,
}

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
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct FeatureRecord {
    pub id: u64,
    pub name: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appearance: Option<Lenient<AppearanceRecord>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(flatten)]
    pub kind: FeatureKindRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct AppearanceRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colour: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub density: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<u8>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub faces: Vec<Lenient<FaceColourRecord>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct FaceColourRecord {
    #[serde(flatten)]
    pub face: FaceRecord,
    pub colour: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FeatureKindRecord {
    Sketch(SketchRecord),
    Extrude(ExtrudeRecord),
    ExtrudeTo(ExtrudeToRecord),
    ExtrudeFrom(ExtrudeFromRecord),
    Revolve(RevolveRecord),
    RevolveTwoAngles(RevolveTwoAnglesRecord),
    RevolveFrom(RevolveFromRecord),
    Fillet(BlendRecord),
    Chamfer(BlendRecord),
    Shell(ShellRecord),
    Combine(CombineRecord),
    Move(MoveRecord),
    Copy(MoveRecord),
    Mirror(MirrorRecord),
    Split(SplitRecord),
    Scale(ScaleRecord),
    Hole(HoleRecord),
    HoleByCircles(HoleRecord),
    HoleScaledByCircles(HoleRecord),
    LinearPattern(Box<LinearPatternRecord>),
    CircularPattern(Box<CircularPatternRecord>),
    Pattern(Box<PatternRecord>),
    Plane(Box<DatumPlaneRecord>),
    Axis(Box<DatumAxisRecord>),
    Point(Box<DatumPointRecord>),
    Remove(RemoveRecord),
    PlaneThrough(Box<PlaneThroughRecord>),
    AxisThrough(Box<AxisThroughRecord>),
    Import(ImportRecord),
    CutSeveral(Box<CutSeveralRecord>),
    PlacedImport(Box<PlacedImportRecord>),
    RevolveOneSide(Box<RevolveOneSideRecord>),
    SteppedHole(Box<SteppedHoleRecord>),
    FeaturePattern(Box<FeaturePatternRecord>),
    MoveAboutCentre(Box<MoveAboutCentreRecord>),
    MoveAboutAxis(Box<MoveAboutAxisRecord>),
    CombineTools(Box<CombineToolsRecord>),
    DrillPointHole(Box<DrillPointHoleRecord>),
    PlaneConstruction(Box<PlaneConstructionRecord>),
    PointConstruction(Box<PointConstructionRecord>),
    DatumConstruction(Box<DatumConstructionRecord>),
    OffsetFace(Box<OffsetFaceRecord>),
    FeatureMirror(Box<FeatureMirrorRecord>),
    Primitive(Box<PrimitiveRecord>),
    Thread(Box<ThreadRecord>),
    SplitAlong(Box<SplitAlongRecord>),
    Mate(Box<MateRecord>),
    CoordinateSystem(Box<CoordinateSystemRecord>),
    MoveInFrame(Box<MoveInFrameRecord>),
    SketchOnFrame(Box<SketchOnFrameRecord>),
    FrameOriginDatum(Box<FrameOriginRecord>),
    ScaleInFrame(Box<MoveInFrameRecord>),
    ImportInFrame(Box<MoveInFrameRecord>),
    ScaledImport(Box<ScaledImportRecord>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct MateRecord {
    pub body: u64,
    pub pair: MatePairRecord,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flipped: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MatePairRecord {
    Faces(Box<FaceMateRecord>),
    Axes(Box<AxisMateRecord>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct FaceMateRecord {
    pub face: Lenient<FaceRecord>,
    pub target: Lenient<PlaneReferenceRecord>,
    pub distance: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct AxisMateRecord {
    pub axis: Lenient<AxisReferenceRecord>,
    pub target: Lenient<AxisReferenceRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct CoordinateSystemRecord {
    pub origin: Lenient<PointReferenceRecord>,
    pub x_axis: Lenient<AxisReferenceRecord>,
    pub plane: Lenient<PlaneReferenceRecord>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reverse_x: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reverse_z: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct MoveInFrameRecord {
    pub feature: FeatureKindRecord,
    pub frame: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ScaledImportRecord {
    pub feature: FeatureKindRecord,
    pub scale: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct FrameOriginRecord {
    pub feature: FeatureKindRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct SketchOnFrameRecord {
    pub feature: FeatureKindRecord,
    pub frame: FramePlaneRecord,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(crate) struct FramePlaneRecord {
    pub frame: u64,
    pub plane: PrincipalPlaneRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ThreadRecord {
    pub body: u64,
    pub face: FaceRecord,
    pub standard: String,
    pub size: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub class: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub left_handed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reversed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct FeatureMirrorRecord {
    pub feature: FeatureKindRecord,
    pub mirrored: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PrimitiveShapeRecord {
    Box {
        length: String,
        width: String,
        height: String,
    },
    Cylinder {
        diameter: String,
        height: String,
    },
    Sphere {
        diameter: String,
    },
    Torus {
        diameter: String,
        tube: String,
    },
    Cone {
        bottom: String,
        top: String,
        height: String,
    },
    Wedge {
        length: String,
        width: String,
        height: String,
        top: String,
    },
    Prism {
        sides: String,
        diameter: String,
        height: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PrimitiveAnchorRecord {
    Corner,
    BaseCentre,
    Centre,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct PrimitiveRecord {
    pub shape: PrimitiveShapeRecord,
    pub plane: Lenient<PlaneReferenceRecord>,
    pub at: [String; 2],
    pub anchor: PrimitiveAnchorRecord,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reversed: bool,
    pub operation: OperationRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct OffsetFaceRecord {
    pub body: u64,
    pub distance: String,
    pub faces: Vec<Lenient<FaceRecord>>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub tangent: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct DrillPointHoleRecord {
    pub feature: FeatureKindRecord,
    pub angle: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct CombineToolsRecord {
    pub feature: FeatureKindRecord,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub more_tools: Vec<u64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub keep_tool: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct MoveAboutAxisRecord {
    pub feature: FeatureKindRecord,
    pub axis: AxisReferenceRecord,
    pub angle: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct MoveAboutCentreRecord {
    pub feature: FeatureKindRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct FeaturePatternRecord {
    pub feature: FeatureKindRecord,
    pub repeated: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct SteppedHoleRecord {
    pub feature: FeatureKindRecord,
    pub steps: Vec<HoleStepRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct HoleStepRecord {
    pub diameter: String,
    pub depth: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct CutSeveralRecord {
    pub feature: FeatureKindRecord,
    pub bodies: Vec<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AxisSideRecord {
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct RevolveOneSideRecord {
    pub feature: FeatureKindRecord,
    pub side: AxisSideRecord,
}

pub(crate) const FEATURE_FIELDS: [&str; 3] = ["hidden", "appearance", "group"];

pub(crate) const FEATURE_KINDS: [&str; 54] = [
    "sketch",
    "extrude",
    "extrude_to",
    "extrude_from",
    "revolve",
    "revolve_two_angles",
    "revolve_from",
    "fillet",
    "chamfer",
    "shell",
    "combine",
    "move",
    "copy",
    "mirror",
    "split",
    "scale",
    "hole",
    "hole_by_circles",
    "linear_pattern",
    "circular_pattern",
    "pattern",
    "plane",
    "axis",
    "point",
    "remove",
    "plane_through",
    "axis_through",
    "import",
    "cut_several",
    "placed_import",
    "hole_scaled_by_circles",
    "revolve_one_side",
    "stepped_hole",
    "feature_pattern",
    "move_about_centre",
    "move_about_axis",
    "combine_tools",
    "drill_point_hole",
    "plane_construction",
    "point_construction",
    "offset_face",
    "feature_mirror",
    "primitive",
    "thread",
    "split_along",
    "mate",
    "coordinate_system",
    "move_in_frame",
    "sketch_on_frame",
    "datum_construction",
    "frame_origin_datum",
    "scale_in_frame",
    "import_in_frame",
    "scaled_import",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ImportRecord {
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub step: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct PlacedImportRecord {
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shares: Option<String>,
    pub offset: [String; 3],
    pub turn: [String; 3],
}

#[derive(Debug, Default)]
pub(crate) struct ImportTexts {
    texts: BTreeMap<String, Arc<str>>,
    solids: BTreeMap<String, Arc<Solid>>,
}

struct StoredShape<'a> {
    source: &'a str,
    path: Option<&'a String>,
    step: Option<&'a str>,
    shares: Option<&'a str>,
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
    Frame(FramePlaneRecord),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AxisReferenceRecord {
    Principal(PrincipalAxisRecord),
    Datum(u64),
    Edge {
        body: u64,
        edge: EdgeRecord,
    },
    Face {
        body: u64,
        face: FaceRecord,
    },
    SketchLine {
        sketch: u64,
        entity: u64,
    },
    Frame {
        frame: u64,
        axis: PrincipalAxisRecord,
    },
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
#[serde(rename_all = "snake_case")]
pub(crate) enum PointReferenceRecord {
    Origin,
    Datum(u64),
    Vertex { body: u64, vertex: String },
    Centre { body: u64, edge: Box<EdgeRecord> },
    SurfaceCentre { body: u64, face: FaceRecord },
    Sketch { sketch: u64, entity: u64 },
    Frame(u64),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct DatumPointRecord {
    pub base: PointReferenceRecord,
    pub offset: [String; 3],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PlaneThroughRecord {
    Points([PointReferenceRecord; 3]),
    Midway([PlaneReferenceRecord; 2]),
    AxisAndPoint {
        axis: AxisReferenceRecord,
        point: PointReferenceRecord,
    },
    NormalTo {
        axis: AxisReferenceRecord,
        point: PointReferenceRecord,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct CurveStationRecord {
    pub body: u64,
    pub edge: EdgeRecord,
    pub distance: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PlaneConstructionRecord {
    Tangent {
        body: u64,
        face: FaceRecord,
        toward: PointReferenceRecord,
    },
    SquareToCurve(CurveStationRecord),
    Lines([AxisReferenceRecord; 2]),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct FaceTangentRecord {
    pub body: u64,
    pub face: FaceRecord,
    pub toward: PointReferenceRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DatumConstructionRecord {
    TangentAt(FaceTangentRecord),
    SquareToFace(FaceTangentRecord),
    EdgeMiddle { body: u64, edge: EdgeRecord },
    FaceCentre { body: u64, face: FaceRecord },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PointConstructionRecord {
    LinesCross([AxisReferenceRecord; 2]),
    AxisAndPlane {
        axis: AxisReferenceRecord,
        plane: PlaneReferenceRecord,
    },
    ThreePlanes([PlaneReferenceRecord; 3]),
    Along(CurveStationRecord),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AxisThroughRecord {
    Points([PointReferenceRecord; 2]),
    NormalTo {
        plane: PlaneReferenceRecord,
        point: PointReferenceRecord,
    },
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

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CombineOperationRecord {
    Join,
    Cut,
    Intersect,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HoleDepthRecord {
    ThroughAll,
    Blind(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HoleStyleRecord {
    Plain,
    Counterbore { diameter: String, depth: String },
    Countersink { diameter: String, angle: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct HoleRecord {
    pub sketch: u64,
    pub body: u64,
    pub diameter: String,
    pub depth: HoleDepthRecord,
    pub style: HoleStyleRecord,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reversed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<SlotRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub standard: Option<HoleStandardRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<TappedThreadRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct TappedThreadRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub left_handed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct SlotRecord {
    pub length: String,
    pub angle: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct HoleStandardRecord {
    pub size: String,
    pub fit: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct MoveRecord {
    pub body: u64,
    pub offset: [String; 3],
    pub turn: [String; 3],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct MirrorRecord {
    pub body: u64,
    pub plane: Lenient<PlaneReferenceRecord>,
    pub keep_original: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct SplitRecord {
    pub body: u64,
    pub plane: Lenient<PlaneReferenceRecord>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flipped: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct SplitAlongRecord {
    pub body: u64,
    pub along: SplitToolRecord,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flipped: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SplitToolRecord {
    Body(u64),
    Sketch(u64),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ScaleRecord {
    pub body: u64,
    pub factor: String,
    pub center: [String; 3],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct RemoveRecord {
    pub body: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct CombineRecord {
    pub body: u64,
    pub tool: u64,
    pub operation: CombineOperationRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct DirectionRecord {
    pub axis: AxisReferenceRecord,
    pub count: String,
    pub spacing: String,
    pub reversed: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub total: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct LinearPatternRecord {
    pub body: u64,
    pub first: DirectionRecord,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub second: Option<DirectionRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PatternShapeRecord {
    Linear(Box<LinearPatternRecord>),
    Circular(Box<CircularPatternRecord>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct PatternRecord {
    pub shape: PatternShapeRecord,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<[u32; 2]>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SolidStartRecord {
    Distance(String),
    Plane(PlaneReferenceRecord),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExtrudeFromExtentRecord {
    Symmetric {
        distance: String,
    },
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
pub(crate) struct ExtrudeFromRecord {
    pub sketch: u64,
    pub regions: RegionsRecord,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region_references: RegionReferencesRecord,
    pub extent: ExtrudeFromExtentRecord,
    pub operation: OperationRecord,
    pub start: Lenient<SolidStartRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RevolveFromExtentRecord {
    Full,
    OneSide { angle: String, reversed: bool },
    Symmetric { angle: String },
    TwoSides { forward: String, backward: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct RevolveFromRecord {
    pub sketch: u64,
    pub regions: RegionsRecord,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region_references: RegionReferencesRecord,
    pub axis: RevolveAxisRecord,
    pub extent: RevolveFromExtentRecord,
    pub operation: OperationRecord,
    pub start: Lenient<SolidStartRecord>,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub projections: Vec<Lenient<ProjectionRecord>>,
    pub next_id: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ProjectionRecord {
    pub entity: u64,
    pub source: ProjectionSourceRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProjectionSourceRecord {
    Edge {
        body: u64,
        edge: EdgeRecord,
    },
    Vertex {
        body: u64,
        vertex: String,
    },
    SketchEntity {
        sketch: u64,
        entity: u64,
    },
    Section {
        body: u64,
        edge: EdgeRecord,
    },
    DatumPlane {
        datum: u64,
        reach: f64,
    },
    PrincipalPlane {
        plane: PrincipalPlaneRecord,
        reach: f64,
    },
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

const DEFAULT_THREAD_DIAMETER: f64 = 8.0;

const ENTITY_KINDS: [&str; 5] = ["point", "line", "circle", "arc", "spline"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ConstraintRecord {
    pub id: u64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub inactive: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<[f64; 2]>,
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
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        diameter: bool,
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
    ArcLength {
        arc: u64,
        value: String,
    },
    Sweep {
        arc: u64,
        value: String,
    },
    Curvature([u64; 2]),
}

const CONSTRAINT_KINDS: [&str; 23] = [
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
    "arc_length",
    "sweep",
    "curvature",
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        owner: Option<OwnerRecord>,
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
    MoveParameter {
        id: u64,
        index: usize,
    },
    SetParameterNote {
        id: u64,
        note: String,
    },
    SetParameterOwner {
        id: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        owner: Option<OwnerRecord>,
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
    SetFeatureGroup {
        id: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        group: Option<String>,
    },
    SetBodyAppearance {
        id: u64,
        appearance: AppearanceRecord,
    },
    SetRollbackBar {
        before: Option<u64>,
    },
    SetPrincipalHidden {
        geometry: PrincipalGeometryRecord,
        hidden: bool,
    },
    SetModelProperties {
        properties: PropertiesRecord,
    },
    SetSavedViews {
        views: ViewsRecord,
    },
    SetSelectionSets {
        sets: SelectionSetsRecord,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        frame: Option<FramePlaneRecord>,
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
    SetSketchProjection {
        feature: u64,
        id: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<ProjectionSourceRecord>,
    },
    AddSketchConstraint {
        feature: u64,
        constraint: ConstraintRecord,
    },
    RemoveSketchConstraint {
        feature: u64,
        id: u64,
    },
    SetSketchConstraintActive {
        feature: u64,
        id: u64,
        active: bool,
    },
    SetSketchLabel {
        feature: u64,
        id: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        offset: Option<[f64; 2]>,
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
        note: parameter.note.clone(),
    }
}

pub(crate) fn feature_record(feature: &Feature) -> FeatureRecord {
    feature_record_sharing(feature, None)
}

pub(crate) fn feature_records(document: &Document) -> impl Iterator<Item = FeatureRecord> + '_ {
    let mut written = BTreeSet::new();
    document.features().map(move |feature| {
        let shares = feature.kind.import().and_then(|import| {
            let digest = blake3::hash(import.step.as_bytes());
            let seen = !written.insert(*digest.as_bytes());
            (seen && !import.placement.is_unmoved()).then(|| digest.to_hex().to_string())
        });
        feature_record_sharing(feature, shares)
    })
}

fn feature_record_sharing(feature: &Feature, shares: Option<String>) -> FeatureRecord {
    FeatureRecord {
        id: feature.id().raw(),
        name: feature.name.clone(),
        hidden: feature.hidden,
        appearance: (!feature.appearance.is_default())
            .then(|| Lenient::Read(appearance_record(&feature.appearance))),
        group: feature.group.clone(),
        kind: match &feature.kind {
            FeatureKind::Import(import) => import_record(import, shares),
            kind => feature_kind_record(kind),
        },
    }
}

fn appearance_record(appearance: &BodyAppearance) -> AppearanceRecord {
    AppearanceRecord {
        colour: appearance.colour.map(Rgb::hex),
        material: appearance.material.clone(),
        density: appearance.density.as_ref().map(Expression::to_stored_text),
        name: appearance.name.clone(),
        opacity: appearance.opacity,
        faces: appearance
            .faces
            .iter()
            .map(|coloured| {
                Lenient::Read(FaceColourRecord {
                    face: face_record(&coloured.face),
                    colour: coloured.colour.hex(),
                })
            })
            .collect(),
    }
}

fn restore_appearance(
    record: &Lenient<AppearanceRecord>,
    name: &str,
    issues: &mut Vec<String>,
) -> BodyAppearance {
    let Lenient::Read(record) = record else {
        issues.push(format!(
            "The colour and material of “{name}” could not be read, so it shows in the default \
             colour with no material."
        ));
        return BodyAppearance::default();
    };
    let colour = record.colour.as_deref().and_then(|text| {
        let colour = Rgb::from_hex(text);
        if colour.is_none() {
            issues.push(format!(
                "The colour of “{name}” could not be read, so it shows in the default colour."
            ));
        }
        colour
    });
    let material = record.material.as_deref().and_then(|text| {
        let material = material_name(text)
            .filter(|material| material.chars().count() <= MAX_MATERIAL_NAME_CHARS);
        if material.is_none() {
            issues.push(format!(
                "The material name of “{name}” could not be used, so it was left out."
            ));
        }
        material
    });
    let density = record.density.as_deref().and_then(|text| {
        let density = Expression::parse_stored(text).ok();
        if density.is_none() {
            issues.push(format!(
                "The density of “{name}” could not be read, so it was left out. Enter it again \
                 to see the body's mass."
            ));
        }
        density
    });
    let body_name = record.name.as_deref().and_then(|text| {
        let body_name = material_name(text)
            .filter(|body_name| body_name.chars().count() <= MAX_BODY_NAME_CHARS);
        if body_name.is_none() {
            issues.push(format!(
                "The body name of “{name}” could not be used, so the body is named after it."
            ));
        }
        body_name
    });
    let opacity = record.opacity.and_then(|opacity| {
        let usable = (MIN_OPACITY_PERCENT..OPAQUE_PERCENT).contains(&opacity);
        if !usable {
            issues.push(format!(
                "The opacity of “{name}” could not be used, so it is drawn solid."
            ));
        }
        usable.then_some(opacity)
    });
    let faces: Vec<FaceColour> = record
        .faces
        .iter()
        .filter_map(|coloured| match coloured {
            Lenient::Read(coloured) => Some(FaceColour {
                face: restore_face(
                    &coloured.face.face,
                    coloured.face.origin,
                    coloured.face.copy,
                    &coloured.face.neighbours,
                )?,
                colour: Rgb::from_hex(&coloured.colour)?,
            }),
            Lenient::Unreadable(_) => None,
        })
        .collect();
    match record.faces.len() - faces.len() {
        0 => {}
        1 => issues.push(format!(
            "The colour of a face of “{name}” could not be read, so it shows in the body's colour."
        )),
        lost => issues.push(format!(
            "The colour of {lost} faces of “{name}” could not be read, so they show in the \
             body's colour."
        )),
    }
    BodyAppearance {
        colour,
        material,
        density,
        name: body_name,
        opacity,
        faces,
    }
}

fn feature_kind_record(kind: &FeatureKind) -> FeatureKindRecord {
    if let FeatureKind::Combine(combine) = kind
        && (!combine.more_tools.is_empty() || combine.keep_tool)
    {
        let alone = Combine::new(combine.body, combine.tool, combine.operation);
        return FeatureKindRecord::CombineTools(Box::new(CombineToolsRecord {
            feature: feature_kind_record(&FeatureKind::Combine(alone)),
            more_tools: combine.more_tools.iter().map(|tool| tool.raw()).collect(),
            keep_tool: combine.keep_tool,
        }));
    }
    if let FeatureKind::Solid(solid) = kind
        && !solid.other_bodies().is_empty()
    {
        let mut alone = solid.clone();
        alone.other_bodies_mut().clear();
        return FeatureKindRecord::CutSeveral(Box::new(CutSeveralRecord {
            feature: feature_kind_record(&FeatureKind::Solid(alone)),
            bodies: solid.other_bodies().iter().map(|body| body.raw()).collect(),
        }));
    }
    if let FeatureKind::Solid(SolidFeature::Revolve(revolve)) = kind
        && let Some(side) = revolve.side
    {
        let whole = Revolve {
            side: None,
            ..revolve.clone()
        };
        return FeatureKindRecord::RevolveOneSide(Box::new(RevolveOneSideRecord {
            feature: feature_kind_record(&FeatureKind::Solid(SolidFeature::Revolve(whole))),
            side: match side {
                AxisSide::Left => AxisSideRecord::Left,
                AxisSide::Right => AxisSideRecord::Right,
            },
        }));
    }
    if let FeatureKind::Datum(datum) = kind
        && datum
            .points()
            .into_iter()
            .any(|point| point.frame().is_some())
    {
        return FeatureKindRecord::FrameOriginDatum(Box::new(FrameOriginRecord {
            feature: kind_record(kind),
        }));
    }
    kind_record(kind)
}

fn kind_record(kind: &FeatureKind) -> FeatureKindRecord {
    match kind {
        FeatureKind::Sketch(sketch) => {
            let record = FeatureKindRecord::Sketch(sketch_record(sketch));
            match sketch.attachment.as_ref().and_then(SketchAttachment::frame) {
                Some((frame, plane)) => {
                    FeatureKindRecord::SketchOnFrame(Box::new(SketchOnFrameRecord {
                        feature: record,
                        frame: frame_plane_record(frame, plane),
                    }))
                }
                None => record,
            }
        }
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
        FeatureKind::Datum(Datum::Axis(axis)) => match axis {
            DatumAxis::Along(reference) => {
                FeatureKindRecord::Axis(Box::new(DatumAxisRecord::Along(axis_record(reference))))
            }
            DatumAxis::Intersection(first, second) => {
                FeatureKindRecord::Axis(Box::new(DatumAxisRecord::Intersection([
                    plane_reference_record(first),
                    plane_reference_record(second),
                ])))
            }
            DatumAxis::Points(first, second) => {
                FeatureKindRecord::AxisThrough(Box::new(AxisThroughRecord::Points([
                    point_record(first),
                    point_record(second),
                ])))
            }
            DatumAxis::NormalTo(plane, point) => {
                FeatureKindRecord::AxisThrough(Box::new(AxisThroughRecord::NormalTo {
                    plane: plane_reference_record(plane),
                    point: point_record(point),
                }))
            }
            DatumAxis::SquareToFace(tangent) => FeatureKindRecord::DatumConstruction(Box::new(
                DatumConstructionRecord::SquareToFace(face_tangent_record(tangent)),
            )),
        },
        FeatureKind::Datum(Datum::Point(point)) => {
            FeatureKindRecord::Point(Box::new(DatumPointRecord {
                base: point_record(&point.base),
                offset: point.offset.clone().map(|value| value.to_stored_text()),
            }))
        }
        FeatureKind::Datum(Datum::PlaneThrough(through)) => match through {
            PlaneThrough::Points(points) => FeatureKindRecord::PlaneThrough(Box::new(
                PlaneThroughRecord::Points(points.clone().map(|point| point_record(&point))),
            )),
            PlaneThrough::Midway(first, second) => {
                FeatureKindRecord::PlaneThrough(Box::new(PlaneThroughRecord::Midway([
                    plane_reference_record(first),
                    plane_reference_record(second),
                ])))
            }
            PlaneThrough::AxisAndPoint(axis, point) => {
                FeatureKindRecord::PlaneThrough(Box::new(PlaneThroughRecord::AxisAndPoint {
                    axis: axis_record(axis),
                    point: point_record(point),
                }))
            }
            PlaneThrough::NormalTo(axis, point) => {
                FeatureKindRecord::PlaneThrough(Box::new(PlaneThroughRecord::NormalTo {
                    axis: axis_record(axis),
                    point: point_record(point),
                }))
            }
            PlaneThrough::Tangent(tangent) => {
                FeatureKindRecord::PlaneConstruction(Box::new(PlaneConstructionRecord::Tangent {
                    body: tangent.body.raw(),
                    face: face_record(&tangent.face),
                    toward: point_record(&tangent.toward),
                }))
            }
            PlaneThrough::SquareToCurve(station) => FeatureKindRecord::PlaneConstruction(Box::new(
                PlaneConstructionRecord::SquareToCurve(station_record(station)),
            )),
            PlaneThrough::Lines(first, second) => {
                FeatureKindRecord::PlaneConstruction(Box::new(PlaneConstructionRecord::Lines([
                    axis_record(first),
                    axis_record(second),
                ])))
            }
            PlaneThrough::TangentAt(tangent) => FeatureKindRecord::DatumConstruction(Box::new(
                DatumConstructionRecord::TangentAt(face_tangent_record(tangent)),
            )),
        },
        FeatureKind::Datum(Datum::PointBy(by)) => {
            let constructed = |record| FeatureKindRecord::PointConstruction(Box::new(record));
            match by {
                PointBy::LinesCross(first, second) => {
                    constructed(PointConstructionRecord::LinesCross([
                        axis_record(first),
                        axis_record(second),
                    ]))
                }
                PointBy::AxisAndPlane(axis, plane) => {
                    constructed(PointConstructionRecord::AxisAndPlane {
                        axis: axis_record(axis),
                        plane: plane_reference_record(plane),
                    })
                }
                PointBy::ThreePlanes(planes) => constructed(PointConstructionRecord::ThreePlanes(
                    planes.each_ref().map(plane_reference_record),
                )),
                PointBy::Along(station) => {
                    constructed(PointConstructionRecord::Along(station_record(station)))
                }
                PointBy::EdgeMiddle { body, edge } => FeatureKindRecord::DatumConstruction(
                    Box::new(DatumConstructionRecord::EdgeMiddle {
                        body: body.raw(),
                        edge: edge_record(edge),
                    }),
                ),
                PointBy::FaceCentre { body, face } => FeatureKindRecord::DatumConstruction(
                    Box::new(DatumConstructionRecord::FaceCentre {
                        body: body.raw(),
                        face: face_record(face),
                    }),
                ),
            }
        }
        FeatureKind::Datum(Datum::Frame(frame)) => {
            FeatureKindRecord::CoordinateSystem(Box::new(CoordinateSystemRecord {
                origin: Lenient::Read(point_record(&frame.origin)),
                x_axis: Lenient::Read(axis_record(&frame.x_axis)),
                plane: Lenient::Read(plane_reference_record(&frame.plane)),
                reverse_x: frame.reverse_x,
                reverse_z: frame.reverse_z,
            }))
        }
        FeatureKind::Shell(shell) => FeatureKindRecord::Shell(ShellRecord {
            body: shell.body.raw(),
            thickness: shell.thickness.to_stored_text(),
            open: shell
                .open
                .iter()
                .map(|face| Lenient::Read(face_record(face)))
                .collect(),
        }),
        FeatureKind::OffsetFace(offset) => {
            FeatureKindRecord::OffsetFace(Box::new(OffsetFaceRecord {
                body: offset.body.raw(),
                distance: offset.distance.to_stored_text(),
                faces: offset
                    .faces
                    .iter()
                    .map(|face| Lenient::Read(face_record(face)))
                    .collect(),
                tangent: offset.tangent,
            }))
        }
        FeatureKind::Remove(remove) => FeatureKindRecord::Remove(RemoveRecord {
            body: remove.body.raw(),
        }),
        FeatureKind::Thread(thread) => FeatureKindRecord::Thread(Box::new(thread_record(thread))),
        FeatureKind::Combine(combine) => FeatureKindRecord::Combine(CombineRecord {
            body: combine.body.raw(),
            tool: combine.tool.raw(),
            operation: match combine.operation {
                CombineOperation::Join => CombineOperationRecord::Join,
                CombineOperation::Cut => CombineOperationRecord::Cut,
                CombineOperation::Intersect => CombineOperationRecord::Intersect,
            },
        }),
        FeatureKind::Move(movement) => {
            let record = MoveRecord {
                body: movement.body.raw(),
                offset: movement.offset.each_ref().map(Expression::to_stored_text),
                turn: movement.turn.each_ref().map(Expression::to_stored_text),
            };
            let feature = if movement.copy {
                FeatureKindRecord::Copy(record)
            } else {
                FeatureKindRecord::Move(record)
            };
            let turned = match &movement.about {
                TurnCentre::Origin => feature,
                TurnCentre::Body => {
                    FeatureKindRecord::MoveAboutCentre(Box::new(MoveAboutCentreRecord { feature }))
                }
                TurnCentre::Axis(turn) => {
                    FeatureKindRecord::MoveAboutAxis(Box::new(MoveAboutAxisRecord {
                        feature,
                        axis: axis_record(&turn.axis),
                        angle: turn.angle.to_stored_text(),
                    }))
                }
            };
            match movement.frame {
                Some(frame) => FeatureKindRecord::MoveInFrame(Box::new(MoveInFrameRecord {
                    feature: turned,
                    frame: frame.raw(),
                })),
                None => turned,
            }
        }
        FeatureKind::Mirror(mirror) => mirror_record(mirror),
        FeatureKind::Primitive(primitive) => {
            FeatureKindRecord::Primitive(Box::new(primitive_record(primitive)))
        }
        FeatureKind::Split(split) => split_record(split),
        FeatureKind::Mate(mate) => FeatureKindRecord::Mate(Box::new(mate_record(mate))),
        FeatureKind::Scale(scale) => {
            let record = FeatureKindRecord::Scale(ScaleRecord {
                body: scale.body.raw(),
                factor: scale.factor.to_stored_text(),
                center: scale.center.each_ref().map(Expression::to_stored_text),
            });
            match scale.frame {
                Some(frame) => FeatureKindRecord::ScaleInFrame(Box::new(MoveInFrameRecord {
                    feature: record,
                    frame: frame.raw(),
                })),
                None => record,
            }
        }
        FeatureKind::Hole(hole) => hole_record(hole),
        FeatureKind::Pattern(pattern) => pattern_record(pattern),
        FeatureKind::Import(import) => import_record(import, None),
    }
}

fn import_record(import: &Import, shares: Option<String>) -> FeatureKindRecord {
    let unscaled = unframed_import_record(import, shares);
    let record = if import.placement.is_unscaled() {
        unscaled
    } else {
        FeatureKindRecord::ScaledImport(Box::new(ScaledImportRecord {
            feature: unscaled,
            scale: import.placement.scale.to_stored_text(),
        }))
    };
    match import.placement.frame {
        Some(frame) => FeatureKindRecord::ImportInFrame(Box::new(MoveInFrameRecord {
            feature: record,
            frame: frame.raw(),
        })),
        None => record,
    }
}

fn unframed_import_record(import: &Import, shares: Option<String>) -> FeatureKindRecord {
    let source = import.source.clone();
    let path = import
        .path
        .as_ref()
        .and_then(|path| path.to_str())
        .map(str::to_owned);
    if import.placement.is_unmoved() {
        return FeatureKindRecord::Import(ImportRecord {
            source,
            path,
            step: import.step.to_string(),
        });
    }
    FeatureKindRecord::PlacedImport(Box::new(PlacedImportRecord {
        source,
        path,
        step: shares.is_none().then(|| import.step.to_string()),
        shares,
        offset: import
            .placement
            .offset
            .each_ref()
            .map(Expression::to_stored_text),
        turn: import
            .placement
            .turn
            .each_ref()
            .map(Expression::to_stored_text),
    }))
}

fn end_record(end: &ExtrudeEnd) -> Lenient<ExtrudeEndRecord> {
    Lenient::Read(match end {
        ExtrudeEnd::Distance(distance) => ExtrudeEndRecord::Distance(distance.to_stored_text()),
        ExtrudeEnd::ThroughAll => ExtrudeEndRecord::ThroughAll,
        ExtrudeEnd::UpToNext => ExtrudeEndRecord::UpToNext,
        ExtrudeEnd::UpToFace(target) => ExtrudeEndRecord::UpToFace(plane_reference_record(target)),
    })
}

fn start_record(start: &SolidStart) -> Lenient<SolidStartRecord> {
    Lenient::Read(match start {
        SolidStart::Distance(distance) => SolidStartRecord::Distance(distance.to_stored_text()),
        SolidStart::Plane(target) => SolidStartRecord::Plane(plane_reference_record(target)),
    })
}

fn extrude_from_record(extrude: &Extrude, start: &SolidStart) -> FeatureKindRecord {
    let extent = match &extrude.extent {
        ExtrudeExtent::Symmetric { distance } => ExtrudeFromExtentRecord::Symmetric {
            distance: distance.to_stored_text(),
        },
        ExtrudeExtent::OneSide { end, reversed } => ExtrudeFromExtentRecord::OneSide {
            end: end_record(end),
            reversed: *reversed,
        },
        ExtrudeExtent::TwoSides { forward, backward } => ExtrudeFromExtentRecord::TwoSides {
            forward: end_record(forward),
            backward: end_record(backward),
        },
    };
    FeatureKindRecord::ExtrudeFrom(ExtrudeFromRecord {
        sketch: extrude.sketch.raw(),
        regions: regions_record(&extrude.regions),
        region_references: region_references_record(&extrude.regions),
        extent,
        operation: operation_record(extrude.operation),
        start: start_record(start),
    })
}

fn extrude_record(extrude: &Extrude) -> FeatureKindRecord {
    if let Some(start @ SolidStart::Plane(_)) = &extrude.start {
        return extrude_from_record(extrude, start);
    }
    let start_text = extrude
        .start
        .as_ref()
        .and_then(SolidStart::distance)
        .map(Expression::to_stored_text);
    let distances = |extent: ExtrudeExtentRecord| {
        FeatureKindRecord::Extrude(ExtrudeRecord {
            sketch: extrude.sketch.raw(),
            regions: regions_record(&extrude.regions),
            region_references: region_references_record(&extrude.regions),
            extent,
            operation: operation_record(extrude.operation),
            start: start_text.clone(),
        })
    };
    let ends = |extent: ExtrudeEndsRecord| {
        FeatureKindRecord::ExtrudeTo(ExtrudeToRecord {
            sketch: extrude.sketch.raw(),
            regions: regions_record(&extrude.regions),
            region_references: region_references_record(&extrude.regions),
            extent,
            operation: operation_record(extrude.operation),
            start: start_text.clone(),
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

fn revolve_from_record(revolve: &Revolve, start: &SolidStart) -> FeatureKindRecord {
    let extent = match &revolve.extent {
        RevolveExtent::Full => RevolveFromExtentRecord::Full,
        RevolveExtent::OneSide { angle, reversed } => RevolveFromExtentRecord::OneSide {
            angle: angle.to_stored_text(),
            reversed: *reversed,
        },
        RevolveExtent::Symmetric { angle } => RevolveFromExtentRecord::Symmetric {
            angle: angle.to_stored_text(),
        },
        RevolveExtent::TwoSides { forward, backward } => RevolveFromExtentRecord::TwoSides {
            forward: forward.to_stored_text(),
            backward: backward.to_stored_text(),
        },
    };
    let axis = match &revolve.axis {
        RevolveAxis::Sketch(line) => RevolveAxisRecord::Sketch(line.raw()),
        RevolveAxis::Model(axis) => RevolveAxisRecord::Model(Box::new(axis_record(axis))),
    };
    FeatureKindRecord::RevolveFrom(RevolveFromRecord {
        sketch: revolve.sketch.raw(),
        regions: regions_record(&revolve.regions),
        region_references: region_references_record(&revolve.regions),
        axis,
        extent,
        operation: operation_record(revolve.operation),
        start: start_record(start),
    })
}

fn revolve_record(revolve: &Revolve) -> FeatureKindRecord {
    let (sketch, regions, region_references, operation) = (
        revolve.sketch.raw(),
        regions_record(&revolve.regions),
        region_references_record(&revolve.regions),
        operation_record(revolve.operation),
    );
    if let Some(start) = &revolve.start {
        return revolve_from_record(revolve, start);
    }
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

fn tapped_thread_record(thread: &TappedThread) -> Option<TappedThreadRecord> {
    (*thread != TappedThread::default()).then(|| TappedThreadRecord {
        class: thread.class.map(|class| class.id().to_owned()),
        left_handed: thread.hand == ThreadHand::Left,
        depth: thread.depth.as_ref().map(Expression::to_stored_text),
    })
}

fn restore_tapped_thread(
    record: Option<&TappedThreadRecord>,
    feature: &str,
    issues: &mut Vec<String>,
) -> TappedThread {
    let Some(record) = record else {
        return TappedThread::default();
    };
    let class = record.class.as_ref().and_then(|id| {
        let class = ThreadFamily::MetricCoarse
            .classes(ThreadSide::Internal)
            .iter()
            .copied()
            .find(|class| class.id() == id);
        if class.is_none() {
            issues.push(format!(
                "The thread class of “{feature}” ({id}) could not be read, so its tapped thread \
                 is class {}.",
                ThreadFamily::MetricCoarse
                    .default_class(ThreadSide::Internal)
                    .id()
            ));
        }
        class
    });
    TappedThread {
        class,
        hand: if record.left_handed {
            ThreadHand::Left
        } else {
            ThreadHand::Right
        },
        depth: record
            .depth
            .as_ref()
            .map(|depth| restore_value(depth, "thread depth", "10 mm", feature, issues)),
    }
}

fn hole_record(hole: &Hole) -> FeatureKindRecord {
    let record = HoleRecord {
        sketch: hole.sketch.raw(),
        body: hole.body.raw(),
        diameter: hole.diameter.to_stored_text(),
        depth: match &hole.depth {
            HoleDepth::ThroughAll => HoleDepthRecord::ThroughAll,
            HoleDepth::Blind(depth) => HoleDepthRecord::Blind(depth.to_stored_text()),
        },
        style: match &hole.style {
            HoleStyle::Plain => HoleStyleRecord::Plain,
            HoleStyle::Counterbore { diameter, depth } => HoleStyleRecord::Counterbore {
                diameter: diameter.to_stored_text(),
                depth: depth.to_stored_text(),
            },
            HoleStyle::Countersink { diameter, angle } => HoleStyleRecord::Countersink {
                diameter: diameter.to_stored_text(),
                angle: angle.to_stored_text(),
            },
            HoleStyle::Stepped(steps) => match steps.first() {
                Some(top) => HoleStyleRecord::Counterbore {
                    diameter: top.diameter.to_stored_text(),
                    depth: top.depth.to_stored_text(),
                },
                None => HoleStyleRecord::Plain,
            },
        },
        reversed: hole.reversed,
        slot: match &hole.shape {
            HoleShape::Round => None,
            HoleShape::Slot { length, angle } => Some(SlotRecord {
                length: length.to_stored_text(),
                angle: angle.to_stored_text(),
            }),
        },
        standard: hole.standard.map(|standard| HoleStandardRecord {
            size: standard.size.name().to_owned(),
            fit: standard.fit.id().to_owned(),
        }),
        thread: tapped_thread_record(&hole.thread),
    };
    let feature = match hole.sizing {
        HoleSizing::Typed => FeatureKindRecord::Hole(record),
        HoleSizing::Circles => FeatureKindRecord::HoleByCircles(record),
        HoleSizing::CirclesAndHeads => FeatureKindRecord::HoleScaledByCircles(record),
    };
    let feature = match &hole.style {
        HoleStyle::Stepped(steps) => FeatureKindRecord::SteppedHole(Box::new(SteppedHoleRecord {
            feature,
            steps: steps
                .iter()
                .map(|step| HoleStepRecord {
                    diameter: step.diameter.to_stored_text(),
                    depth: step.depth.to_stored_text(),
                })
                .collect(),
        })),
        HoleStyle::Plain | HoleStyle::Counterbore { .. } | HoleStyle::Countersink { .. } => feature,
    };
    match &hole.bottom {
        HoleBottom::Flat => feature,
        HoleBottom::DrillPoint(angle) => {
            FeatureKindRecord::DrillPointHole(Box::new(DrillPointHoleRecord {
                feature,
                angle: angle.to_stored_text(),
            }))
        }
    }
}

fn pattern_record(pattern: &Pattern) -> FeatureKindRecord {
    let body = pattern.body.raw();
    let shape = match &pattern.kind {
        PatternKind::Linear { first, second } => {
            PatternShapeRecord::Linear(Box::new(LinearPatternRecord {
                body,
                first: direction_record(first),
                second: second.as_ref().map(direction_record),
            }))
        }
        PatternKind::Circular(circular) => {
            PatternShapeRecord::Circular(Box::new(CircularPatternRecord {
                body,
                axis: axis_record(&circular.axis),
                count: circular.count.to_stored_text(),
                angle: circular.angle.to_stored_text(),
                reversed: circular.reversed,
            }))
        }
    };
    let skipped: Vec<[u32; 2]> = pattern.skipped.iter().copied().collect();
    let total = match &shape {
        PatternShapeRecord::Linear(linear) => std::iter::once(&linear.first)
            .chain(&linear.second)
            .any(|direction| direction.total),
        PatternShapeRecord::Circular(_) => false,
    };
    let record = match (shape, skipped.is_empty() && !total) {
        (PatternShapeRecord::Linear(linear), true) => FeatureKindRecord::LinearPattern(linear),
        (PatternShapeRecord::Circular(circular), true) => {
            FeatureKindRecord::CircularPattern(circular)
        }
        (shape, false) => FeatureKindRecord::Pattern(Box::new(PatternRecord { shape, skipped })),
    };
    if pattern.repeated.is_empty() {
        return record;
    }
    FeatureKindRecord::FeaturePattern(Box::new(FeaturePatternRecord {
        feature: record,
        repeated: pattern
            .repeated
            .iter()
            .map(|feature| feature.raw())
            .collect(),
    }))
}

fn mirror_record(mirror: &Mirror) -> FeatureKindRecord {
    let record = FeatureKindRecord::Mirror(MirrorRecord {
        body: mirror.body.raw(),
        plane: Lenient::Read(plane_reference_record(&mirror.plane)),
        keep_original: mirror.keep_original,
    });
    if !mirror.mirrors_features() {
        return record;
    }
    FeatureKindRecord::FeatureMirror(Box::new(FeatureMirrorRecord {
        feature: record,
        mirrored: mirror
            .mirrored
            .iter()
            .map(|feature| feature.raw())
            .collect(),
    }))
}

fn direction_record(direction: &LinearDirection) -> DirectionRecord {
    DirectionRecord {
        axis: axis_record(&direction.axis),
        count: direction.count.to_stored_text(),
        spacing: direction.spacing.to_stored_text(),
        reversed: direction.reversed,
        total: direction.measured == LinearSpacing::Total,
    }
}

pub(crate) fn edge_record(edge: &EdgeReference) -> EdgeRecord {
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

fn properties_record_of(properties: &ModelProperties) -> PropertiesRecord {
    PropertiesRecord {
        title: properties.title.clone(),
        part_number: properties.part_number.clone(),
        revision: properties.revision.clone(),
        author: properties.author.clone(),
        organisation: properties.organisation.clone(),
        description: properties.description.clone(),
        notes: properties.notes.clone(),
    }
}

pub(crate) fn properties_record(document: &Document) -> Option<PropertiesRecord> {
    let properties = document.properties();
    (!properties.is_empty()).then(|| properties_record_of(properties))
}

pub(crate) fn restore_properties(
    record: PropertiesRecord,
    issues: &mut Vec<String>,
) -> ModelProperties {
    let mut properties = ModelProperties {
        title: record.title,
        part_number: record.part_number,
        revision: record.revision,
        author: record.author,
        organisation: record.organisation,
        description: record.description,
        notes: record.notes,
    }
    .normalized();
    for property in ModelProperty::ALL {
        let field = properties.get_mut(property);
        let limit = property.max_chars();
        if field.chars().count() > limit {
            *field = field
                .chars()
                .take(limit)
                .collect::<String>()
                .trim_end()
                .to_owned();
            issues.push(format!(
                "The model's {} was longer than {limit} characters, so its end was cut off.",
                property.in_sentence()
            ));
        }
    }
    properties
}

fn view_record_of(view: &SavedView) -> ViewRecord {
    ViewRecord {
        target: view.target.to_array(),
        orientation: view.orientation.to_array(),
        distance: view.distance,
    }
}

fn saved_view_of(record: ViewRecord) -> SavedView {
    SavedView {
        target: Point3::from_array(record.target),
        orientation: Rotation3::from_array(record.orientation),
        distance: record.distance,
    }
}

fn views_record_of(views: &SavedViews) -> ViewsRecord {
    ViewsRecord {
        named: views
            .named
            .iter()
            .map(|named| {
                Lenient::Read(NamedViewRecord {
                    name: named.name.clone(),
                    view: view_record_of(&named.view),
                })
            })
            .collect(),
        home: views
            .home
            .as_ref()
            .map(|home| Lenient::Read(view_record_of(home))),
    }
}

pub(crate) fn owner_record(owner: &ParameterOwner) -> OwnerRecord {
    match owner {
        ParameterOwner::Feature { feature, value } => OwnerRecord::Feature {
            feature: feature.raw(),
            value: value.clone(),
        },
        ParameterOwner::Dimension { sketch, constraint } => OwnerRecord::Dimension {
            sketch: sketch.raw(),
            constraint: constraint.raw(),
        },
    }
}

pub(crate) fn restore_owner(record: OwnerRecord) -> ParameterOwner {
    match record {
        OwnerRecord::Feature { feature, value } => ParameterOwner::Feature {
            feature: FeatureId::from_raw(feature),
            value,
        },
        OwnerRecord::Dimension { sketch, constraint } => ParameterOwner::Dimension {
            sketch: FeatureId::from_raw(sketch),
            constraint: ConstraintId::from_raw(constraint),
        },
    }
}

pub(crate) fn named_values_record(document: &Document) -> Option<NamedValuesRecord> {
    let values: Vec<Lenient<NamedValueRecord>> = document
        .parameters()
        .iter()
        .filter_map(|parameter| {
            parameter.owner.as_ref().map(|owner| {
                Lenient::Read(NamedValueRecord {
                    parameter: parameter.id().raw(),
                    owner: owner_record(owner),
                })
            })
        })
        .collect();
    (!values.is_empty()).then_some(NamedValuesRecord { values })
}

pub(crate) fn views_record(document: &Document) -> Option<ViewsRecord> {
    let views = document.saved_views();
    (!views.is_empty()).then(|| views_record_of(views))
}

fn fitting_view_name(name: &str, taken: &SavedViews, issues: &mut Vec<String>) -> String {
    let mut name = view_name(name);
    if name.is_empty() {
        let numbered = taken.unused_name();
        issues.push(format!(
            "A saved view had no name, so it is called “{numbered}”."
        ));
        return numbered;
    }
    if name.chars().count() > MAX_VIEW_NAME_CHARS {
        name = name
            .chars()
            .take(MAX_VIEW_NAME_CHARS)
            .collect::<String>()
            .trim_end()
            .to_owned();
        issues.push(format!(
            "The name of the saved view “{name}” was longer than {MAX_VIEW_NAME_CHARS} characters, \
             so its end was cut off."
        ));
    }
    if !taken.is_taken(&name) {
        return name;
    }
    let room = MAX_VIEW_NAME_CHARS - 6;
    let base: String = name.chars().take(room).collect();
    let numbered = (2..)
        .map(|number| format!("{base} {number}"))
        .find(|candidate| !taken.is_taken(candidate))
        .unwrap_or_else(|| name.clone());
    issues.push(format!(
        "Two saved views were called “{name}”, so one is called “{numbered}”."
    ));
    numbered
}

pub(crate) fn restore_views(record: ViewsRecord, issues: &mut Vec<String>) -> SavedViews {
    let mut views = SavedViews::default();
    for named in record.named {
        let Lenient::Read(named) = named else {
            issues.push("A saved view could not be read, so it was left out.".to_owned());
            continue;
        };
        if views.named.len() >= MAX_SAVED_VIEWS {
            issues.push(format!(
                "A model keeps at most {MAX_SAVED_VIEWS} saved views, so the rest were left out."
            ));
            break;
        }
        let view = saved_view_of(named.view);
        if !view.is_usable() {
            issues.push(format!(
                "The saved view “{}” is not a view the camera can show, so it was left out.",
                named.name
            ));
            continue;
        }
        let name = fitting_view_name(&named.name, &views, issues);
        views.named.push(NamedView {
            name,
            view: view.normalized(),
        });
    }
    views.home = match record.home {
        None => None,
        Some(Lenient::Read(home)) => {
            let home = saved_view_of(home);
            if !home.is_usable() {
                issues.push(format!(
                    "The {HOME_VIEW_NAME} view is not a view the camera can show, so the default \
                     one is used."
                ));
            }
            home.is_usable().then(|| home.normalized())
        }
        Some(Lenient::Unreadable(_)) => {
            issues.push(format!(
                "The {HOME_VIEW_NAME} view could not be read, so the default one is used."
            ));
            None
        }
    };
    views
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
        PlaneReference::Frame { frame, plane } => {
            PlaneReferenceRecord::Frame(frame_plane_record(*frame, *plane))
        }
    }
}

fn frame_plane_record(frame: FeatureId, plane: PrincipalPlane) -> FramePlaneRecord {
    FramePlaneRecord {
        frame: frame.raw(),
        plane: principal_plane_record(plane),
    }
}

fn station_record(station: &CurveStation) -> CurveStationRecord {
    CurveStationRecord {
        body: station.body.raw(),
        edge: edge_record(&station.edge),
        distance: station.distance.to_stored_text(),
    }
}

fn point_record(reference: &PointReference) -> PointReferenceRecord {
    match reference {
        PointReference::Origin => PointReferenceRecord::Origin,
        PointReference::Datum(feature) => PointReferenceRecord::Datum(feature.raw()),
        PointReference::Vertex { body, vertex } => PointReferenceRecord::Vertex {
            body: body.raw(),
            vertex: hex(vertex.digest()),
        },
        PointReference::Centre { body, edge } => PointReferenceRecord::Centre {
            body: body.raw(),
            edge: Box::new(edge_record(edge)),
        },
        PointReference::SurfaceCentre { body, face } => PointReferenceRecord::SurfaceCentre {
            body: body.raw(),
            face: face_record(face),
        },
        PointReference::Sketch { sketch, entity } => PointReferenceRecord::Sketch {
            sketch: sketch.raw(),
            entity: entity.raw(),
        },
        PointReference::Frame(frame) => PointReferenceRecord::Frame(frame.raw()),
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
        AxisReference::Sketch { sketch, entity } => AxisReferenceRecord::SketchLine {
            sketch: sketch.raw(),
            entity: entity.raw(),
        },
        AxisReference::Frame { frame, axis } => AxisReferenceRecord::Frame {
            frame: frame.raw(),
            axis: principal_axis_record(*axis),
        },
    }
}

pub(crate) fn face_record(face: &FaceReference) -> FaceRecord {
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
            .map(|(id, constraint)| {
                Lenient::Read(constraint_record(
                    id,
                    constraint,
                    !sketch.is_active(id),
                    sketch.label_offset(id),
                ))
            })
            .collect(),
        projections: feature
            .projections
            .iter()
            .map(|(entity, source)| {
                Lenient::Read(ProjectionRecord {
                    entity: entity.raw(),
                    source: projection_source_record(source),
                })
            })
            .collect(),
        next_id: sketch.next_id(),
    }
}

fn projection_source_record(source: &ProjectionSource) -> ProjectionSourceRecord {
    match source {
        ProjectionSource::Edge { body, edge } => ProjectionSourceRecord::Edge {
            body: body.raw(),
            edge: edge_record(edge),
        },
        ProjectionSource::Vertex { body, vertex } => ProjectionSourceRecord::Vertex {
            body: body.raw(),
            vertex: hex(vertex.digest()),
        },
        ProjectionSource::SketchEntity { sketch, entity } => ProjectionSourceRecord::SketchEntity {
            sketch: sketch.raw(),
            entity: entity.raw(),
        },
        ProjectionSource::Section { body, edge } => ProjectionSourceRecord::Section {
            body: body.raw(),
            edge: edge_record(edge),
        },
        ProjectionSource::DatumPlane { datum, reach } => ProjectionSourceRecord::DatumPlane {
            datum: datum.raw(),
            reach: *reach,
        },
        ProjectionSource::PrincipalPlane { plane, reach } => {
            ProjectionSourceRecord::PrincipalPlane {
                plane: principal_plane_record(*plane),
                reach: *reach,
            }
        }
    }
}

fn restore_projection_source(record: &ProjectionSourceRecord) -> Option<ProjectionSource> {
    Some(match record {
        ProjectionSourceRecord::Edge { body, edge } => ProjectionSource::Edge {
            body: FeatureId::from_raw(*body),
            edge: restore_edge(edge)?,
        },
        ProjectionSourceRecord::Vertex { body, vertex } => ProjectionSource::Vertex {
            body: FeatureId::from_raw(*body),
            vertex: VertexName::from_digest(restore_digest(vertex)?),
        },
        ProjectionSourceRecord::SketchEntity { sketch, entity } => ProjectionSource::SketchEntity {
            sketch: FeatureId::from_raw(*sketch),
            entity: EntityId::from_raw(*entity),
        },
        ProjectionSourceRecord::Section { body, edge } => ProjectionSource::Section {
            body: FeatureId::from_raw(*body),
            edge: restore_edge(edge)?,
        },
        ProjectionSourceRecord::DatumPlane { datum, reach } => ProjectionSource::DatumPlane {
            datum: FeatureId::from_raw(*datum),
            reach: Some(*reach).filter(|reach| reach.is_finite() && *reach > 0.0)?,
        },
        ProjectionSourceRecord::PrincipalPlane { plane, reach } => {
            ProjectionSource::PrincipalPlane {
                plane: restore_principal_plane(*plane),
                reach: Some(*reach).filter(|reach| reach.is_finite() && *reach > 0.0)?,
            }
        }
    })
}

fn restore_projections(
    record: &SketchRecord,
    sketch: &mut Sketch,
    feature: &str,
    issues: &mut Vec<String>,
) -> BTreeMap<EntityId, ProjectionSource> {
    let mut projections = BTreeMap::new();
    for projection in &record.projections {
        let Lenient::Read(projection) = projection else {
            issues.push(format!(
                "In “{feature}”, the source of projected geometry could not be read, so it stays \
                 where it was saved as ordinary geometry."
            ));
            continue;
        };
        let id = EntityId::from_raw(projection.entity);
        let label = sketch.entity_label(id);
        let Some(entity) = sketch.entity(id) else {
            continue;
        };
        let Some(source) = restore_projection_source(&projection.source) else {
            issues.push(format!(
                "In “{feature}”, the source {label} was projected from could not be read, so it \
                 stays where it was saved as ordinary geometry."
            ));
            continue;
        };
        let marked: Vec<EntityId> = entity.points().into_iter().chain([id]).collect();
        for each in marked {
            if sketch.set_projected(each, true).is_err() {
                issues.push(format!(
                    "In “{feature}”, part of {label} is missing, so it may move away from the \
                     geometry it was projected from."
                ));
            }
        }
        projections.insert(id, source);
    }
    projections
}

fn entity_record(id: EntityId, entity: &Entity, construction: bool) -> EntityRecord {
    EntityRecord {
        id: id.raw(),
        construction,
        kind: entity_kind_record(entity),
    }
}

fn constraint_record(
    id: ConstraintId,
    constraint: &Constraint,
    inactive: bool,
    label: Option<Vector2>,
) -> ConstraintRecord {
    ConstraintRecord {
        id: id.raw(),
        inactive,
        label: label.map(|offset| offset.to_array()),
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
            diameter: false,
        },
        Constraint::AxisDiameter { point, axis, value } => ConstraintKindRecord::Distance {
            from: point.raw(),
            to: axis.raw(),
            value: radius_of_diameter(value).to_stored_text(),
            diameter: true,
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
        Constraint::Midpoint { point, curve } => ConstraintKindRecord::Midpoint {
            point: point.raw(),
            line: curve.raw(),
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
        Constraint::ArcLength { arc, value } => ConstraintKindRecord::ArcLength {
            arc: arc.raw(),
            value: value.to_stored_text(),
        },
        Constraint::Sweep { arc, value } => ConstraintKindRecord::Sweep {
            arc: arc.raw(),
            value: value.to_stored_text(),
        },
        Constraint::Curvature(a, b) => ConstraintKindRecord::Curvature(pair(a, b)),
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
            owner: parameter.owner.as_ref().map(owner_record),
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
        Edit::MoveParameter { id, index } => EditRecord::MoveParameter {
            id: id.raw(),
            index: *index,
        },
        Edit::SetParameterNote { id, note } => EditRecord::SetParameterNote {
            id: id.raw(),
            note: note.clone(),
        },
        Edit::SetParameterOwner { id, owner } => EditRecord::SetParameterOwner {
            id: id.raw(),
            owner: owner.as_ref().map(owner_record),
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
        Edit::SetFeatureGroup { id, group } => EditRecord::SetFeatureGroup {
            id: id.raw(),
            group: group.clone(),
        },
        Edit::SetFeatureSuppressed { id, suppressed } => EditRecord::SetFeatureSuppressed {
            id: id.raw(),
            suppressed: *suppressed,
        },
        Edit::SetBodyAppearance { id, appearance } => EditRecord::SetBodyAppearance {
            id: id.raw(),
            appearance: appearance_record(appearance),
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
        Edit::SetModelProperties { properties } => EditRecord::SetModelProperties {
            properties: properties_record_of(properties),
        },
        Edit::SetSavedViews { views } => EditRecord::SetSavedViews {
            views: views_record_of(views),
        },
        Edit::SetSelectionSets { sets } => EditRecord::SetSelectionSets {
            sets: selection_sets_record_of(sets),
        },
        Edit::SetFeatureKind { id, kind } => EditRecord::SetFeatureKind {
            feature: FeatureRecord {
                id: id.raw(),
                name: String::new(),
                hidden: false,
                appearance: None,
                group: None,
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
            frame: attachment
                .as_ref()
                .and_then(SketchAttachment::frame)
                .map(|(frame, plane)| frame_plane_record(frame, plane)),
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
        Edit::SetSketchProjection {
            feature,
            id,
            source,
        } => EditRecord::SetSketchProjection {
            feature: feature.raw(),
            id: id.raw(),
            source: source.as_ref().map(projection_source_record),
        },
        Edit::AddSketchConstraint {
            feature,
            id,
            constraint,
            inactive,
            label,
        } => EditRecord::AddSketchConstraint {
            feature: feature.raw(),
            constraint: constraint_record(*id, constraint, *inactive, *label),
        },
        Edit::SetSketchConstraintActive {
            feature,
            id,
            active,
        } => EditRecord::SetSketchConstraintActive {
            feature: feature.raw(),
            id: id.raw(),
            active: *active,
        },
        Edit::SetSketchLabel {
            feature,
            id,
            offset,
        } => EditRecord::SetSketchLabel {
            feature: feature.raw(),
            id: id.raw(),
            offset: offset.map(|offset| offset.to_array()),
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
        EditRecord::InsertParameter {
            index,
            parameter,
            owner,
        } => {
            let mut restored = Parameter::new(
                ParameterId::from_raw(parameter.id),
                parameter.name,
                parse(&parameter.expression)?,
            )
            .with_note(parameter.note);
            restored.owner = owner.map(restore_owner);
            Edit::InsertParameter {
                index,
                parameter: restored,
            }
        }
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
        EditRecord::MoveParameter { id, index } => Edit::MoveParameter {
            id: ParameterId::from_raw(id),
            index,
        },
        EditRecord::SetParameterNote { id, note } => Edit::SetParameterNote {
            id: ParameterId::from_raw(id),
            note,
        },
        EditRecord::SetParameterOwner { id, owner } => Edit::SetParameterOwner {
            id: ParameterId::from_raw(id),
            owner: owner.map(restore_owner),
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
        EditRecord::SetFeatureGroup { id, group } => Edit::SetFeatureGroup {
            id: FeatureId::from_raw(id),
            group,
        },
        EditRecord::SetFeatureSuppressed { id, suppressed } => Edit::SetFeatureSuppressed {
            id: FeatureId::from_raw(id),
            suppressed,
        },
        EditRecord::SetBodyAppearance { id, appearance } => {
            let mut issues = Vec::new();
            let appearance = restore_appearance(&Lenient::Read(appearance), "", &mut issues);
            if !issues.is_empty() {
                return None;
            }
            Edit::SetBodyAppearance {
                id: FeatureId::from_raw(id),
                appearance,
            }
        }
        EditRecord::SetRollbackBar { before } => Edit::SetRollbackBar {
            bar: restore_rollback(before),
        },
        EditRecord::SetPrincipalHidden { geometry, hidden } => Edit::SetPrincipalHidden {
            geometry: restore_principal(geometry),
            hidden,
        },
        EditRecord::SetModelProperties { properties } => Edit::SetModelProperties {
            properties: Box::new(restore_properties(properties, &mut Vec::new())),
        },
        EditRecord::SetSavedViews { views } => Edit::SetSavedViews {
            views: Box::new(restore_views(views, &mut Vec::new())),
        },
        EditRecord::SetSelectionSets { sets } => Edit::SetSelectionSets {
            sets: Box::new(restore_selection_sets(sets, &mut Vec::new())),
        },
        EditRecord::SetFeatureKind { feature } => {
            let mut issues = Vec::new();
            let kind = restore_kind(&feature.kind, "", &mut ImportTexts::default(), &mut issues);
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
            frame,
        } => Edit::SetSketchPlacement {
            feature: FeatureId::from_raw(feature),
            plane: restore_plane(plane)?,
            attachment: match (attachment, datum, frame) {
                (Some(record), _, _) => Some(SketchAttachment::Face(restore_attachment(&record)?)),
                (None, Some(datum), _) => Some(SketchAttachment::Datum(FeatureId::from_raw(datum))),
                (None, None, Some(frame)) => Some(SketchAttachment::Frame {
                    frame: FeatureId::from_raw(frame.frame),
                    plane: restore_principal_plane(frame.plane),
                }),
                (None, None, None) => None,
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
        EditRecord::SetSketchProjection {
            feature,
            id,
            source,
        } => Edit::SetSketchProjection {
            feature: FeatureId::from_raw(feature),
            id: EntityId::from_raw(id),
            source: match source {
                Some(record) => Some(restore_projection_source(&record)?),
                None => None,
            },
        },
        EditRecord::AddSketchConstraint {
            feature,
            constraint,
        } => Edit::AddSketchConstraint {
            feature: FeatureId::from_raw(feature),
            id: ConstraintId::from_raw(constraint.id),
            constraint: constraint_from_record(&constraint.kind, |text, _| parse(text))?,
            inactive: constraint.inactive,
            label: constraint.label.map(Vector2::from_array),
        },
        EditRecord::SetSketchConstraintActive {
            feature,
            id,
            active,
        } => Edit::SetSketchConstraintActive {
            feature: FeatureId::from_raw(feature),
            id: ConstraintId::from_raw(id),
            active,
        },
        EditRecord::SetSketchLabel {
            feature,
            id,
            offset,
        } => Edit::SetSketchLabel {
            feature: FeatureId::from_raw(feature),
            id: ConstraintId::from_raw(id),
            offset: offset.map(Vector2::from_array),
        },
        EditRecord::RemoveSketchConstraint { feature, id } => Edit::RemoveSketchConstraint {
            feature: FeatureId::from_raw(feature),
            id: ConstraintId::from_raw(id),
        },
    })
}

pub(crate) fn restore_feature(record: &FeatureRecord, issues: &mut Vec<String>) -> Feature {
    restore_feature_sharing(record, &mut ImportTexts::default(), issues)
}

pub(crate) fn restore_feature_sharing(
    record: &FeatureRecord,
    texts: &mut ImportTexts,
    issues: &mut Vec<String>,
) -> Feature {
    let name = if record.name.trim().is_empty() {
        let fallback = format!("Feature {}", record.id);
        issues.push(format!(
            "A feature had no name, so it was named “{fallback}”."
        ));
        fallback
    } else {
        record.name.clone()
    };
    let kind = restore_kind(&record.kind, &name, texts, issues);
    let mut feature = Feature::new(FeatureId::from_raw(record.id), name, kind);
    feature.hidden = record.hidden;
    if let Some(appearance) = &record.appearance {
        feature.appearance = restore_appearance(appearance, &feature.name, issues);
    }
    feature.group = record
        .group
        .as_deref()
        .and_then(|group| restore_group(group, &feature.name, issues));
    feature
}

fn restore_group(group: &str, feature: &str, issues: &mut Vec<String>) -> Option<String> {
    let name = group_name(group)?;
    if name.chars().count() <= MAX_GROUP_NAME_CHARS {
        return Some(name);
    }
    issues.push(format!(
        "The name of the group holding “{feature}” was longer than {MAX_GROUP_NAME_CHARS} \
         characters, so it was cut."
    ));
    Some(name.chars().take(MAX_GROUP_NAME_CHARS).collect())
}

fn restore_kind(
    record: &FeatureKindRecord,
    name: &str,
    texts: &mut ImportTexts,
    issues: &mut Vec<String>,
) -> FeatureKind {
    match record {
        FeatureKindRecord::CutSeveral(several) => {
            let mut kind = restore_kind(&several.feature, name, texts, issues);
            match &mut kind {
                FeatureKind::Solid(solid) => {
                    *solid.other_bodies_mut() = several
                        .bodies
                        .iter()
                        .copied()
                        .map(FeatureId::from_raw)
                        .collect();
                }
                _ => issues.push(format!(
                    "“{name}” listed other bodies to cut, but it is not an extrusion or a \
                     revolution, so they were left out."
                )),
            }
            kind
        }
        FeatureKindRecord::DrillPointHole(pointed) => {
            let mut kind = restore_kind(&pointed.feature, name, texts, issues);
            match &mut kind {
                FeatureKind::Hole(hole) => {
                    hole.bottom = HoleBottom::DrillPoint(restore_value(
                        &pointed.angle,
                        "drill point angle",
                        "118 deg",
                        name,
                        issues,
                    ));
                }
                _ => issues.push(format!(
                    "“{name}” was to end in a drill point, but it is not a hole, so it ends flat."
                )),
            }
            kind
        }
        FeatureKindRecord::SteppedHole(stepped) => {
            let mut kind = restore_kind(&stepped.feature, name, texts, issues);
            match &mut kind {
                FeatureKind::Hole(hole) => {
                    hole.style = HoleStyle::Stepped(
                        stepped
                            .steps
                            .iter()
                            .enumerate()
                            .map(|(index, step)| {
                                let number = index + 1;
                                HoleStep {
                                    diameter: restore_value(
                                        &step.diameter,
                                        &format!("step {number} diameter"),
                                        "10 mm",
                                        name,
                                        issues,
                                    ),
                                    depth: restore_value(
                                        &step.depth,
                                        &format!("step {number} depth"),
                                        "3 mm",
                                        name,
                                        issues,
                                    ),
                                }
                            })
                            .collect(),
                    );
                }
                _ => issues.push(format!(
                    "“{name}” listed the steps of a stepped hole, but it is not a hole, so they \
                     were left out."
                )),
            }
            kind
        }
        FeatureKindRecord::FeaturePattern(repeating) => {
            let mut kind = restore_kind(&repeating.feature, name, texts, issues);
            match &mut kind {
                FeatureKind::Pattern(pattern) => {
                    let mut repeated = Vec::new();
                    for raw in &repeating.repeated {
                        let feature = FeatureId::from_raw(*raw);
                        if !repeated.contains(&feature) {
                            repeated.push(feature);
                        }
                    }
                    pattern.repeated = repeated;
                }
                _ => issues.push(format!(
                    "“{name}” listed features to repeat, but it is not a pattern, so they were \
                     left out."
                )),
            }
            kind
        }
        FeatureKindRecord::FeatureMirror(mirroring) => {
            let mut kind = restore_kind(&mirroring.feature, name, texts, issues);
            match &mut kind {
                FeatureKind::Mirror(mirror) => {
                    let mut mirrored = Vec::new();
                    for raw in &mirroring.mirrored {
                        let feature = FeatureId::from_raw(*raw);
                        if !mirrored.contains(&feature) {
                            mirrored.push(feature);
                        }
                    }
                    mirror.mirrored = mirrored;
                }
                _ => issues.push(format!(
                    "“{name}” listed features to mirror, but it is not a mirror, so they were \
                     left out."
                )),
            }
            kind
        }
        FeatureKindRecord::MoveAboutCentre(centred) => {
            let mut kind = restore_kind(&centred.feature, name, texts, issues);
            match &mut kind {
                FeatureKind::Move(movement) => movement.about = TurnCentre::Body,
                _ => issues.push(format!(
                    "“{name}” was to turn about its body's centre, but it is not a move, so that \
                     was left out."
                )),
            }
            kind
        }
        FeatureKindRecord::MoveInFrame(framed) => {
            let mut kind = restore_kind(&framed.feature, name, texts, issues);
            match &mut kind {
                FeatureKind::Move(movement) => {
                    movement.frame = Some(FeatureId::from_raw(framed.frame));
                }
                _ => issues.push(format!(
                    "“{name}” was to move in a coordinate system, but it is not a move, so that \
                     was left out."
                )),
            }
            kind
        }
        FeatureKindRecord::FrameOriginDatum(framed) => {
            let kind = restore_kind(&framed.feature, name, texts, issues);
            if !matches!(kind, FeatureKind::Datum(_)) {
                issues.push(format!(
                    "“{name}” was to use the origin of a coordinate system, but it is not a \
                     datum, so that was left out."
                ));
            }
            kind
        }
        FeatureKindRecord::ScaleInFrame(framed) => {
            let mut kind = restore_kind(&framed.feature, name, texts, issues);
            match &mut kind {
                FeatureKind::Scale(scale) => {
                    scale.frame = Some(FeatureId::from_raw(framed.frame));
                }
                _ => issues.push(format!(
                    "“{name}” was to scale about a centre in a coordinate system, but it is not \
                     a scale, so that was left out."
                )),
            }
            kind
        }
        FeatureKindRecord::ScaledImport(scaled) => {
            let mut kind = restore_kind(&scaled.feature, name, texts, issues);
            match &mut kind {
                FeatureKind::Import(import) => {
                    import.placement.scale =
                        restore_value(&scaled.scale, "import scale", "1", name, issues);
                }
                _ => issues.push(format!(
                    "“{name}” was to be scaled as it is imported, but it is not an import, so \
                     that was left out."
                )),
            }
            kind
        }
        FeatureKindRecord::ImportInFrame(framed) => {
            let mut kind = restore_kind(&framed.feature, name, texts, issues);
            match &mut kind {
                FeatureKind::Import(import) => {
                    import.placement.frame = Some(FeatureId::from_raw(framed.frame));
                }
                _ => issues.push(format!(
                    "“{name}” was to be placed in a coordinate system, but it is not an import, \
                     so that was left out."
                )),
            }
            kind
        }
        FeatureKindRecord::SketchOnFrame(placed) => {
            let mut kind = restore_kind(&placed.feature, name, texts, issues);
            match &mut kind {
                FeatureKind::Sketch(sketch) => {
                    sketch.attachment = Some(SketchAttachment::Frame {
                        frame: FeatureId::from_raw(placed.frame.frame),
                        plane: restore_principal_plane(placed.frame.plane),
                    });
                }
                _ => issues.push(format!(
                    "“{name}” was to lie on a plane of a coordinate system, but it is not a \
                     sketch, so that was left out."
                )),
            }
            kind
        }
        FeatureKindRecord::MoveAboutAxis(about) => {
            let mut kind = restore_kind(&about.feature, name, texts, issues);
            match &mut kind {
                FeatureKind::Move(movement) => {
                    movement.about = restore_axis_turn(about, name, issues);
                }
                _ => issues.push(format!(
                    "“{name}” was to turn about an axis, but it is not a move, so that was left \
                     out."
                )),
            }
            kind
        }
        FeatureKindRecord::RevolveOneSide(one_side) => {
            let mut kind = restore_kind(&one_side.feature, name, texts, issues);
            match &mut kind {
                FeatureKind::Solid(SolidFeature::Revolve(revolve)) => {
                    revolve.side = Some(match one_side.side {
                        AxisSideRecord::Left => AxisSide::Left,
                        AxisSideRecord::Right => AxisSide::Right,
                    });
                }
                _ => issues.push(format!(
                    "“{name}” was to keep one side of its revolution axis, but it is not a \
                     revolution, so it keeps its whole profile."
                )),
            }
            kind
        }
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
            let mut restored = restore_sketch(sketch, name, issues);
            let projections = restore_projections(sketch, &mut restored, name, issues);
            FeatureKind::Sketch(SketchFeature {
                sketch: restored,
                attachment,
                projections,
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
                start: extrude.start.as_deref().map(|text| {
                    SolidStart::Distance(restore_value(text, "start offset", "0 mm", name, issues))
                }),
                other_bodies: Vec::new(),
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
                start: extrude.start.as_deref().map(|text| {
                    SolidStart::Distance(restore_value(text, "start offset", "0 mm", name, issues))
                }),
                other_bodies: Vec::new(),
            }))
        }
        FeatureKindRecord::ExtrudeFrom(extrude) => {
            let extent = match &extrude.extent {
                ExtrudeFromExtentRecord::Symmetric { distance } => ExtrudeExtent::Symmetric {
                    distance: restore_value(distance, "distance", "10 mm", name, issues),
                },
                ExtrudeFromExtentRecord::OneSide { end, reversed } => ExtrudeExtent::OneSide {
                    end: restore_end(end, ("end", "distance"), name, issues),
                    reversed: *reversed,
                },
                ExtrudeFromExtentRecord::TwoSides { forward, backward } => {
                    ExtrudeExtent::TwoSides {
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
                    }
                }
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
                start: restore_start(&extrude.start, name, issues),
                other_bodies: Vec::new(),
            }))
        }
        FeatureKindRecord::RevolveFrom(revolve) => {
            let mut value =
                |text: &str, what: &str| restore_value(text, what, "180 deg", name, issues);
            let extent = match &revolve.extent {
                RevolveFromExtentRecord::Full => RevolveExtent::Full,
                RevolveFromExtentRecord::OneSide { angle, reversed } => RevolveExtent::OneSide {
                    angle: value(angle, "angle"),
                    reversed: *reversed,
                },
                RevolveFromExtentRecord::Symmetric { angle } => RevolveExtent::Symmetric {
                    angle: value(angle, "angle"),
                },
                RevolveFromExtentRecord::TwoSides { forward, backward } => {
                    RevolveExtent::TwoSides {
                        forward: value(forward, "forward angle"),
                        backward: value(backward, "backward angle"),
                    }
                }
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
                start: restore_start(&revolve.start, name, issues),
                other_bodies: Vec::new(),
                side: None,
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
                start: None,
                other_bodies: Vec::new(),
                side: None,
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
                start: None,
                other_bodies: Vec::new(),
                side: None,
            }))
        }
        FeatureKindRecord::Fillet(record) => {
            FeatureKind::Blend(restore_blend(record, BlendKind::Fillet, name, issues))
        }
        FeatureKindRecord::Chamfer(record) => {
            FeatureKind::Blend(restore_blend(record, BlendKind::Chamfer, name, issues))
        }
        FeatureKindRecord::Shell(record) => FeatureKind::Shell(restore_shell(record, name, issues)),
        FeatureKindRecord::OffsetFace(record) => {
            FeatureKind::OffsetFace(restore_offset_face(record, name, issues))
        }
        FeatureKindRecord::Thread(record) => {
            FeatureKind::Thread(restore_thread(record, name, issues))
        }
        FeatureKindRecord::Remove(record) => FeatureKind::Remove(Remove {
            body: FeatureId::from_raw(record.body),
        }),
        FeatureKindRecord::Combine(record) => FeatureKind::Combine(Combine::new(
            FeatureId::from_raw(record.body),
            FeatureId::from_raw(record.tool),
            match record.operation {
                CombineOperationRecord::Join => CombineOperation::Join,
                CombineOperationRecord::Cut => CombineOperation::Cut,
                CombineOperationRecord::Intersect => CombineOperation::Intersect,
            },
        )),
        FeatureKindRecord::CombineTools(tools) => {
            let mut kind = restore_kind(&tools.feature, name, texts, issues);
            match &mut kind {
                FeatureKind::Combine(combine) => {
                    let mut more_tools = Vec::new();
                    for raw in &tools.more_tools {
                        let tool = FeatureId::from_raw(*raw);
                        if tool != combine.tool && !more_tools.contains(&tool) {
                            more_tools.push(tool);
                        }
                    }
                    combine.more_tools = more_tools;
                    combine.keep_tool = tools.keep_tool;
                }
                _ => issues.push(format!(
                    "“{name}” listed tool bodies to combine, but it is not a combine, so they \
                     were left out."
                )),
            }
            kind
        }
        FeatureKindRecord::Move(record) => {
            FeatureKind::Move(restore_move(record, false, name, issues))
        }
        FeatureKindRecord::Copy(record) => {
            FeatureKind::Move(restore_move(record, true, name, issues))
        }
        FeatureKindRecord::Mirror(record) => {
            FeatureKind::Mirror(restore_mirror(record, name, issues))
        }
        FeatureKindRecord::Split(record) => FeatureKind::Split(restore_split(record, name, issues)),
        FeatureKindRecord::SplitAlong(record) => FeatureKind::Split(restore_split_along(record)),
        FeatureKindRecord::Mate(record) => FeatureKind::Mate(restore_mate(record, name, issues)),
        FeatureKindRecord::Primitive(record) => {
            FeatureKind::Primitive(restore_primitive(record, name, issues))
        }
        FeatureKindRecord::Scale(record) => FeatureKind::Scale(restore_scale(record, name, issues)),
        FeatureKindRecord::Hole(record) => {
            FeatureKind::Hole(restore_hole(record, HoleSizing::Typed, name, issues))
        }
        FeatureKindRecord::HoleByCircles(record) => {
            FeatureKind::Hole(restore_hole(record, HoleSizing::Circles, name, issues))
        }
        FeatureKindRecord::HoleScaledByCircles(record) => FeatureKind::Hole(restore_hole(
            record,
            HoleSizing::CirclesAndHeads,
            name,
            issues,
        )),
        FeatureKindRecord::LinearPattern(record) => {
            FeatureKind::from(restore_linear_pattern(record, name, issues))
        }
        FeatureKindRecord::CircularPattern(record) => {
            FeatureKind::from(restore_circular_pattern(record, name, issues))
        }
        FeatureKindRecord::Pattern(record) => {
            FeatureKind::from(restore_pattern(record, name, issues))
        }
        FeatureKindRecord::Plane(record) => {
            FeatureKind::Datum(Datum::Plane(restore_datum_plane(record, name, issues)))
        }
        FeatureKindRecord::Axis(record) => {
            FeatureKind::Datum(Datum::Axis(restore_datum_axis(record, name, issues)))
        }
        FeatureKindRecord::Point(record) => {
            FeatureKind::Datum(Datum::Point(restore_datum_point(record, name, issues)))
        }
        FeatureKindRecord::PlaneThrough(record) => {
            FeatureKind::Datum(restore_plane_through(record, name, issues))
        }
        FeatureKindRecord::AxisThrough(record) => {
            FeatureKind::Datum(Datum::Axis(restore_axis_through(record, name, issues)))
        }
        FeatureKindRecord::PlaneConstruction(record) => {
            FeatureKind::Datum(restore_plane_construction(record, name, issues))
        }
        FeatureKindRecord::PointConstruction(record) => {
            FeatureKind::Datum(restore_point_construction(record, name, issues))
        }
        FeatureKindRecord::CoordinateSystem(record) => FeatureKind::Datum(Datum::Frame(Box::new(
            restore_coordinate_system(record, name, issues),
        ))),
        FeatureKindRecord::DatumConstruction(record) => {
            FeatureKind::Datum(restore_datum_construction(record, name, issues))
        }
        FeatureKindRecord::Import(record) => FeatureKind::Import(restore_import(
            &StoredShape {
                source: &record.source,
                path: record.path.as_ref(),
                step: Some(&record.step),
                shares: None,
            },
            name,
            texts,
            issues,
        )),
        FeatureKindRecord::PlacedImport(record) => {
            FeatureKind::Import(restore_placed_import(record, name, texts, issues))
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

fn restore_start(
    record: &Lenient<SolidStartRecord>,
    name: &str,
    issues: &mut Vec<String>,
) -> Option<SolidStart> {
    match record {
        Lenient::Read(SolidStartRecord::Distance(text)) => Some(SolidStart::Distance(
            restore_value(text, "start offset", "0 mm", name, issues),
        )),
        Lenient::Read(SolidStartRecord::Plane(target)) => match restore_plane_reference(target) {
            Some(target) => Some(SolidStart::Plane(target)),
            None => {
                issues.push(format!(
                    "The face or plane that “{name}” starts from could not be read, so it starts \
                     at its sketch plane."
                ));
                None
            }
        },
        Lenient::Unreadable(_) => {
            issues.push(format!(
                "Where “{name}” starts could not be read, so it starts at its sketch plane."
            ));
            None
        }
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

fn restore_import(
    record: &StoredShape<'_>,
    name: &str,
    texts: &mut ImportTexts,
    issues: &mut Vec<String>,
) -> Import {
    let source = record.source;
    let (digest, text) = match (record.step, record.shares) {
        (Some(step), _) => {
            let digest = blake3::hash(step.as_bytes()).to_hex().to_string();
            let text = texts
                .texts
                .entry(digest.clone())
                .or_insert_with(|| Arc::from(step))
                .clone();
            (digest, text)
        }
        (None, Some(shares)) => match texts.texts.get(shares) {
            Some(text) => (shares.to_owned(), text.clone()),
            None => {
                issues.push(format!(
                    "The shape of “{name}”, imported from “{source}”, was kept with another \
                     import that could not be read, so the feature has no shape."
                ));
                (String::new(), Arc::from(""))
            }
        },
        (None, None) => (String::new(), Arc::from("")),
    };
    let solid = match texts.solids.get(&digest) {
        Some(solid) => solid.clone(),
        None => match crate::step_cache::first_solid(&text) {
            Ok(read) => {
                let read = Arc::new(read.unwrap_or_default());
                if !digest.is_empty() {
                    texts.solids.insert(digest, read.clone());
                }
                read
            }
            Err(error) => {
                issues.push(format!(
                    "The shape of “{name}”, imported from “{source}”, could not be read \
                     ({error}), so the feature has no shape."
                ));
                Arc::new(Solid::default())
            }
        },
    };
    let import = Import::shared(source, solid, text);
    match record.path {
        Some(path) => import.from_file(PathBuf::from(path)),
        None => import,
    }
}

fn restore_placed_import(
    record: &PlacedImportRecord,
    name: &str,
    texts: &mut ImportTexts,
    issues: &mut Vec<String>,
) -> Import {
    let shape = StoredShape {
        source: &record.source,
        path: record.path.as_ref(),
        step: record.step.as_deref(),
        shares: record.shares.as_deref(),
    };
    let import = restore_import(&shape, name, texts, issues);
    let mut read = |texts: &[String; 3], what: &str, fallback: &str| {
        texts
            .each_ref()
            .map(|text| restore_value(text, what, fallback, name, issues))
    };
    let placement = BodyPlacement {
        offset: read(&record.offset, "placement distance", "0 mm"),
        turn: read(&record.turn, "placement turn", "0 deg"),
        ..BodyPlacement::default()
    };
    import.placed(placement)
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
        PlaneReferenceRecord::Frame(frame) => PlaneReference::Frame {
            frame: FeatureId::from_raw(frame.frame),
            plane: restore_principal_plane(frame.plane),
        },
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
        AxisReferenceRecord::SketchLine { sketch, entity } => AxisReference::Sketch {
            sketch: FeatureId::from_raw(*sketch),
            entity: EntityId::from_raw(*entity),
        },
        AxisReferenceRecord::Frame { frame, axis } => AxisReference::Frame {
            frame: FeatureId::from_raw(*frame),
            axis: restore_principal_axis(*axis),
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

fn restore_coordinate_system(
    record: &CoordinateSystemRecord,
    feature: &str,
    issues: &mut Vec<String>,
) -> DatumFrame {
    let world = DatumFrame::world();
    let origin = match &record.origin {
        Lenient::Read(origin) => restore_point(origin),
        Lenient::Unreadable(_) => None,
    };
    let x_axis = match &record.x_axis {
        Lenient::Read(axis) => restore_axis(axis),
        Lenient::Unreadable(_) => None,
    };
    let plane = match &record.plane {
        Lenient::Read(plane) => restore_plane_reference(plane),
        Lenient::Unreadable(_) => None,
    };
    DatumFrame {
        origin: origin.unwrap_or_else(|| {
            issues.push(format!(
                "The origin of “{feature}” could not be read, so it stands at the origin."
            ));
            world.origin
        }),
        x_axis: x_axis.unwrap_or_else(|| {
            issues.push(format!(
                "The X axis of “{feature}” could not be read, so it runs along the X axis."
            ));
            world.x_axis
        }),
        plane: plane.unwrap_or_else(|| {
            issues.push(format!(
                "The plane of “{feature}” could not be read, so it lies on the XY plane."
            ));
            world.plane
        }),
        reverse_x: record.reverse_x,
        reverse_z: record.reverse_z,
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

fn restore_point(record: &PointReferenceRecord) -> Option<PointReference> {
    Some(match record {
        PointReferenceRecord::Origin => PointReference::Origin,
        PointReferenceRecord::Datum(feature) => {
            PointReference::Datum(FeatureId::from_raw(*feature))
        }
        PointReferenceRecord::Vertex { body, vertex } => PointReference::Vertex {
            body: FeatureId::from_raw(*body),
            vertex: VertexName::from_digest(restore_digest(vertex)?),
        },
        PointReferenceRecord::Centre { body, edge } => PointReference::Centre {
            body: FeatureId::from_raw(*body),
            edge: Box::new(restore_edge(edge)?),
        },
        PointReferenceRecord::SurfaceCentre { body, face } => PointReference::SurfaceCentre {
            body: FeatureId::from_raw(*body),
            face: restore_face(&face.face, face.origin, face.copy, &face.neighbours)?,
        },
        PointReferenceRecord::Sketch { sketch, entity } => PointReference::Sketch {
            sketch: FeatureId::from_raw(*sketch),
            entity: EntityId::from_raw(*entity),
        },
        PointReferenceRecord::Frame(frame) => PointReference::Frame(FeatureId::from_raw(*frame)),
    })
}

fn restore_datum_point(
    record: &DatumPointRecord,
    feature: &str,
    issues: &mut Vec<String>,
) -> DatumPoint {
    let base = restore_point(&record.base).unwrap_or_else(|| {
        issues.push(format!(
            "The point “{feature}” is placed at could not be read, so it is at the origin."
        ));
        PointReference::Origin
    });
    let [x, y, z] = &record.offset;
    DatumPoint {
        base,
        offset: [
            restore_value(x, "offset along X", "0 mm", feature, issues),
            restore_value(y, "offset along Y", "0 mm", feature, issues),
            restore_value(z, "offset along Z", "0 mm", feature, issues),
        ],
    }
}

fn restore_plane_through(
    record: &PlaneThroughRecord,
    feature: &str,
    issues: &mut Vec<String>,
) -> Datum {
    let restored = match record {
        PlaneThroughRecord::Points([first, second, third]) => restore_point(first)
            .zip(restore_point(second))
            .zip(restore_point(third))
            .map(|((first, second), third)| PlaneThrough::Points([first, second, third])),
        PlaneThroughRecord::Midway([first, second]) => restore_plane_reference(first)
            .zip(restore_plane_reference(second))
            .map(|(first, second)| PlaneThrough::Midway(first, second)),
        PlaneThroughRecord::AxisAndPoint { axis, point } => restore_axis(axis)
            .zip(restore_point(point))
            .map(|(axis, point)| PlaneThrough::AxisAndPoint(axis, point)),
        PlaneThroughRecord::NormalTo { axis, point } => restore_axis(axis)
            .zip(restore_point(point))
            .map(|(axis, point)| PlaneThrough::NormalTo(axis, point)),
    };
    restored.map_or_else(
        || {
            issues.push(format!(
                "What “{feature}” passes through could not be read, so it is the XY plane."
            ));
            Datum::Plane(DatumPlane {
                base: PlaneReference::Principal(PrincipalPlane::Xy),
                rotation: None,
                offset: Expression::Measure(0.0, Unit::Millimetre),
            })
        },
        Datum::PlaneThrough,
    )
}

fn restore_station(
    record: &CurveStationRecord,
    feature: &str,
    issues: &mut Vec<String>,
) -> Option<CurveStation> {
    Some(CurveStation {
        body: FeatureId::from_raw(record.body),
        edge: Box::new(restore_edge(&record.edge)?),
        distance: restore_value(&record.distance, "distance", "0 mm", feature, issues),
    })
}

fn restore_plane_construction(
    record: &PlaneConstructionRecord,
    feature: &str,
    issues: &mut Vec<String>,
) -> Datum {
    let restored = match record {
        PlaneConstructionRecord::Tangent { body, face, toward } => {
            restore_face(&face.face, face.origin, face.copy, &face.neighbours)
                .zip(restore_point(toward))
                .map(|(face, toward)| {
                    PlaneThrough::Tangent(Box::new(FaceTangent {
                        body: FeatureId::from_raw(*body),
                        face,
                        toward,
                    }))
                })
        }
        PlaneConstructionRecord::SquareToCurve(station) => {
            restore_station(station, feature, issues)
                .map(|station| PlaneThrough::SquareToCurve(Box::new(station)))
        }
        PlaneConstructionRecord::Lines([first, second]) => restore_axis(first)
            .zip(restore_axis(second))
            .map(|(first, second)| PlaneThrough::Lines(first, second)),
    };
    restored.map_or_else(
        || {
            issues.push(format!(
                "What “{feature}” is placed by could not be read, so it is the XY plane."
            ));
            Datum::Plane(DatumPlane {
                base: PlaneReference::Principal(PrincipalPlane::Xy),
                rotation: None,
                offset: Expression::Measure(0.0, Unit::Millimetre),
            })
        },
        Datum::PlaneThrough,
    )
}

fn face_tangent_record(tangent: &FaceTangent) -> FaceTangentRecord {
    FaceTangentRecord {
        body: tangent.body.raw(),
        face: face_record(&tangent.face),
        toward: point_record(&tangent.toward),
    }
}

fn restore_face_tangent(record: &FaceTangentRecord) -> Option<Box<FaceTangent>> {
    let face = &record.face;
    let restored = restore_face(&face.face, face.origin, face.copy, &face.neighbours)
        .zip(restore_point(&record.toward))
        .map(|(face, toward)| FaceTangent {
            body: FeatureId::from_raw(record.body),
            face,
            toward,
        });
    restored.map(Box::new)
}

fn restore_datum_construction(
    record: &DatumConstructionRecord,
    feature: &str,
    issues: &mut Vec<String>,
) -> Datum {
    let restored = match record {
        DatumConstructionRecord::TangentAt(tangent) => restore_face_tangent(tangent)
            .map(|tangent| Datum::PlaneThrough(PlaneThrough::TangentAt(tangent))),
        DatumConstructionRecord::SquareToFace(tangent) => restore_face_tangent(tangent)
            .map(|tangent| Datum::Axis(DatumAxis::SquareToFace(tangent))),
        DatumConstructionRecord::EdgeMiddle { body, edge } => restore_edge(edge).map(|edge| {
            Datum::PointBy(PointBy::EdgeMiddle {
                body: FeatureId::from_raw(*body),
                edge: Box::new(edge),
            })
        }),
        DatumConstructionRecord::FaceCentre { body, face } => {
            restore_face(&face.face, face.origin, face.copy, &face.neighbours).map(|face| {
                Datum::PointBy(PointBy::FaceCentre {
                    body: FeatureId::from_raw(*body),
                    face,
                })
            })
        }
    };
    restored.unwrap_or_else(|| match record {
        DatumConstructionRecord::TangentAt(_) => {
            issues.push(format!(
                "What “{feature}” is placed by could not be read, so it is the XY plane."
            ));
            Datum::Plane(DatumPlane {
                base: PlaneReference::Principal(PrincipalPlane::Xy),
                rotation: None,
                offset: Expression::Measure(0.0, Unit::Millimetre),
            })
        }
        DatumConstructionRecord::SquareToFace(_) => {
            issues.push(format!(
                "What “{feature}” runs through could not be read, so it runs along the Z axis."
            ));
            Datum::Axis(DatumAxis::Along(AxisReference::Principal(PrincipalAxis::Z)))
        }
        DatumConstructionRecord::EdgeMiddle { .. } | DatumConstructionRecord::FaceCentre { .. } => {
            issues.push(format!(
                "What “{feature}” is placed by could not be read, so it is at the origin."
            ));
            Datum::Point(DatumPoint {
                base: PointReference::Origin,
                offset: [0, 1, 2].map(|_| Expression::Measure(0.0, Unit::Millimetre)),
            })
        }
    })
}

fn restore_point_construction(
    record: &PointConstructionRecord,
    feature: &str,
    issues: &mut Vec<String>,
) -> Datum {
    let restored = match record {
        PointConstructionRecord::LinesCross([first, second]) => restore_axis(first)
            .zip(restore_axis(second))
            .map(|(first, second)| PointBy::LinesCross(first, second)),
        PointConstructionRecord::AxisAndPlane { axis, plane } => restore_axis(axis)
            .zip(restore_plane_reference(plane))
            .map(|(axis, plane)| PointBy::AxisAndPlane(axis, plane)),
        PointConstructionRecord::ThreePlanes([first, second, third]) => {
            restore_plane_reference(first)
                .zip(restore_plane_reference(second))
                .zip(restore_plane_reference(third))
                .map(|((first, second), third)| PointBy::ThreePlanes([first, second, third]))
        }
        PointConstructionRecord::Along(station) => restore_station(station, feature, issues)
            .map(|station| PointBy::Along(Box::new(station))),
    };
    restored.map_or_else(
        || {
            issues.push(format!(
                "What “{feature}” is placed by could not be read, so it is at the origin."
            ));
            Datum::Point(DatumPoint {
                base: PointReference::Origin,
                offset: [0, 1, 2].map(|_| Expression::Measure(0.0, Unit::Millimetre)),
            })
        },
        Datum::PointBy,
    )
}

fn restore_axis_through(
    record: &AxisThroughRecord,
    feature: &str,
    issues: &mut Vec<String>,
) -> DatumAxis {
    let restored = match record {
        AxisThroughRecord::Points([first, second]) => restore_point(first)
            .zip(restore_point(second))
            .map(|(first, second)| DatumAxis::Points(first, second)),
        AxisThroughRecord::NormalTo { plane, point } => restore_plane_reference(plane)
            .zip(restore_point(point))
            .map(|(plane, point)| DatumAxis::NormalTo(plane, point)),
    };
    restored.unwrap_or_else(|| {
        issues.push(format!(
            "What “{feature}” runs through could not be read, so it runs along the Z axis."
        ));
        DatumAxis::Along(AxisReference::Principal(PrincipalAxis::Z))
    })
}

fn restore_pattern(record: &PatternRecord, feature: &str, issues: &mut Vec<String>) -> Pattern {
    let mut pattern = match &record.shape {
        PatternShapeRecord::Linear(linear) => restore_linear_pattern(linear, feature, issues),
        PatternShapeRecord::Circular(circular) => {
            restore_circular_pattern(circular, feature, issues)
        }
    };
    pattern.skipped = record
        .skipped
        .iter()
        .copied()
        .filter(|instance| {
            *instance != ORIGINAL_INSTANCE
                && instance.iter().all(|step| *step < MAX_PATTERN_INSTANCES)
        })
        .collect();
    pattern
}

fn measured(record: &DirectionRecord) -> LinearSpacing {
    if record.total {
        LinearSpacing::Total
    } else {
        LinearSpacing::BetweenCopies
    }
}

fn restore_direction(
    record: &DirectionRecord,
    which: &str,
    feature: &str,
    issues: &mut Vec<String>,
) -> Option<LinearDirection> {
    let axis = restore_axis(&record.axis)?;
    Some(LinearDirection {
        measured: measured(record),
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
            measured: measured(&record.first),
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
    Pattern::new(
        FeatureId::from_raw(record.body),
        PatternKind::Linear { first, second },
    )
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
    Pattern::new(
        FeatureId::from_raw(record.body),
        PatternKind::Circular(CircularPattern {
            axis,
            count: restore_value(&record.count, "count", "1", feature, issues),
            angle: restore_value(&record.angle, "angle", "360 deg", feature, issues),
            reversed: record.reversed,
        }),
    )
}

fn restore_hole(
    record: &HoleRecord,
    sizing: HoleSizing,
    feature: &str,
    issues: &mut Vec<String>,
) -> Hole {
    let mut value = |text: &str, what: &str, fallback: &str| {
        restore_value(text, what, fallback, feature, issues)
    };
    let diameter = value(&record.diameter, "diameter", "5 mm");
    let depth = match &record.depth {
        HoleDepthRecord::ThroughAll => HoleDepth::ThroughAll,
        HoleDepthRecord::Blind(depth) => HoleDepth::Blind(value(depth, "depth", "10 mm")),
    };
    let style = match &record.style {
        HoleStyleRecord::Plain => HoleStyle::Plain,
        HoleStyleRecord::Counterbore { diameter, depth } => HoleStyle::Counterbore {
            diameter: value(diameter, "counterbore diameter", "10 mm"),
            depth: value(depth, "counterbore depth", "3 mm"),
        },
        HoleStyleRecord::Countersink { diameter, angle } => HoleStyle::Countersink {
            diameter: value(diameter, "countersink diameter", "10 mm"),
            angle: value(angle, "countersink angle", "90 deg"),
        },
    };
    let shape = match &record.slot {
        None => HoleShape::Round,
        Some(slot) => HoleShape::Slot {
            length: value(&slot.length, "slot length", "10 mm"),
            angle: value(&slot.angle, "slot angle", "0 deg"),
        },
    };
    let standard = record.standard.as_ref().and_then(|standard| {
        let read = MetricSize::from_name(&standard.size)
            .zip(HoleFit::from_id(&standard.fit))
            .filter(|(size, fit)| size.offers(*fit))
            .map(|(size, fit)| HoleStandard { size, fit });
        if read.is_none() {
            issues.push(format!(
                "The standard size of “{feature}” ({} {}) is not one this version of caditor \
                 knows, so its sizes are kept as typed values.",
                standard.size, standard.fit
            ));
        }
        read
    });
    Hole {
        sketch: FeatureId::from_raw(record.sketch),
        body: FeatureId::from_raw(record.body),
        diameter,
        depth,
        style,
        reversed: record.reversed,
        shape,
        standard,
        sizing,
        bottom: HoleBottom::Flat,
        thread: restore_tapped_thread(record.thread.as_ref(), feature, issues),
    }
}

fn restore_axis_turn(
    record: &MoveAboutAxisRecord,
    feature: &str,
    issues: &mut Vec<String>,
) -> TurnCentre {
    let Some(axis) = restore_axis(&record.axis) else {
        issues.push(format!(
            "The axis “{feature}” turns about could not be read, so it turns about its body's \
             centre instead."
        ));
        return TurnCentre::Body;
    };
    TurnCentre::Axis(Box::new(AxisTurn {
        axis,
        angle: restore_value(&record.angle, "angle", "0 deg", feature, issues),
    }))
}

fn restore_move(record: &MoveRecord, copy: bool, feature: &str, issues: &mut Vec<String>) -> Move {
    let mut read = |texts: &[String; 3], what: &str, fallback: &str| {
        texts
            .each_ref()
            .map(|text| restore_value(text, what, fallback, feature, issues))
    };
    Move {
        body: FeatureId::from_raw(record.body),
        offset: read(&record.offset, "distance", "0 mm"),
        turn: read(&record.turn, "turn", "0 deg"),
        copy,
        about: TurnCentre::Origin,
        frame: None,
    }
}

fn restore_mirror(record: &MirrorRecord, feature: &str, issues: &mut Vec<String>) -> Mirror {
    let plane = match &record.plane {
        Lenient::Read(plane) => restore_plane_reference(plane),
        Lenient::Unreadable(_) => None,
    };
    let plane = plane.unwrap_or_else(|| {
        issues.push(format!(
            "The plane “{feature}” mirrors across could not be read, so it mirrors across the YZ \
             plane."
        ));
        PlaneReference::Principal(PrincipalPlane::Yz)
    });
    Mirror {
        body: FeatureId::from_raw(record.body),
        plane,
        keep_original: record.keep_original,
        mirrored: Vec::new(),
    }
}

fn split_record(split: &Split) -> FeatureKindRecord {
    let along = match &split.along {
        SplitAlong::Plane(plane) => {
            return FeatureKindRecord::Split(SplitRecord {
                body: split.body.raw(),
                plane: Lenient::Read(plane_reference_record(plane)),
                flipped: split.flipped,
            });
        }
        SplitAlong::Body(tool) => SplitToolRecord::Body(tool.raw()),
        SplitAlong::Sketch(sketch) => SplitToolRecord::Sketch(sketch.raw()),
    };
    FeatureKindRecord::SplitAlong(Box::new(SplitAlongRecord {
        body: split.body.raw(),
        along,
        flipped: split.flipped,
    }))
}

fn mate_record(mate: &Mate) -> MateRecord {
    MateRecord {
        body: mate.body.raw(),
        pair: match &mate.pair {
            MatePair::Faces(faces) => MatePairRecord::Faces(Box::new(FaceMateRecord {
                face: Lenient::Read(face_record(&faces.face)),
                target: Lenient::Read(plane_reference_record(&faces.target)),
                distance: faces.distance.to_stored_text(),
            })),
            MatePair::Axes(axes) => MatePairRecord::Axes(Box::new(AxisMateRecord {
                axis: Lenient::Read(axis_record(&axes.axis)),
                target: Lenient::Read(axis_record(&axes.target)),
            })),
        },
        flipped: mate.flipped,
    }
}

fn restore_mate(record: &MateRecord, feature: &str, issues: &mut Vec<String>) -> Mate {
    let pair = match &record.pair {
        MatePairRecord::Faces(faces) => {
            let FaceMateRecord {
                face,
                target,
                distance,
            } = faces.as_ref();
            let face = match face {
                Lenient::Read(face) => {
                    restore_face(&face.face, face.origin, face.copy, &face.neighbours)
                }
                Lenient::Unreadable(_) => None,
            };
            let face = face.unwrap_or_else(|| {
                issues.push(format!(
                    "The face “{feature}” mates could not be read; choose it again."
                ));
                FaceReference::new(FaceName::from_digest(0), None, [])
            });
            let target = match target {
                Lenient::Read(target) => restore_plane_reference(target),
                Lenient::Unreadable(_) => None,
            };
            let target = target.unwrap_or_else(|| {
                issues.push(format!(
                    "The face “{feature}” mates onto could not be read, so it mates onto the XY \
                     plane."
                ));
                PlaneReference::Principal(PrincipalPlane::Xy)
            });
            MatePair::Faces(Box::new(FaceMate {
                face,
                target,
                distance: restore_value(distance, "distance", "0 mm", feature, issues),
            }))
        }
        MatePairRecord::Axes(axes) => {
            let AxisMateRecord { axis, target } = axes.as_ref();
            let mut axis_or_z = |axis: &Lenient<AxisReferenceRecord>, role: &str| {
                let read = match axis {
                    Lenient::Read(axis) => restore_axis(axis),
                    Lenient::Unreadable(_) => None,
                };
                read.unwrap_or_else(|| {
                    issues.push(format!(
                        "The axis “{feature}” {role} could not be read, so it is the Z axis."
                    ));
                    AxisReference::Principal(PrincipalAxis::Z)
                })
            };
            let axis = axis_or_z(axis, "mates");
            let target = axis_or_z(target, "mates onto");
            MatePair::Axes(Box::new(AxisMate { axis, target }))
        }
    };
    Mate {
        body: FeatureId::from_raw(record.body),
        pair,
        flipped: record.flipped,
    }
}

fn restore_split_along(record: &SplitAlongRecord) -> Split {
    Split {
        body: FeatureId::from_raw(record.body),
        along: match record.along {
            SplitToolRecord::Body(tool) => SplitAlong::Body(FeatureId::from_raw(tool)),
            SplitToolRecord::Sketch(sketch) => SplitAlong::Sketch(FeatureId::from_raw(sketch)),
        },
        flipped: record.flipped,
    }
}

fn restore_split(record: &SplitRecord, feature: &str, issues: &mut Vec<String>) -> Split {
    let plane = match &record.plane {
        Lenient::Read(plane) => restore_plane_reference(plane),
        Lenient::Unreadable(_) => None,
    };
    let plane = plane.unwrap_or_else(|| {
        issues.push(format!(
            "The plane “{feature}” splits along could not be read, so it splits along the YZ \
             plane."
        ));
        PlaneReference::Principal(PrincipalPlane::Yz)
    });
    Split {
        body: FeatureId::from_raw(record.body),
        along: SplitAlong::Plane(plane),
        flipped: record.flipped,
    }
}

fn primitive_record(primitive: &Primitive) -> PrimitiveRecord {
    let text = Expression::to_stored_text;
    PrimitiveRecord {
        shape: match &primitive.shape {
            PrimitiveShape::Box {
                length,
                width,
                height,
            } => PrimitiveShapeRecord::Box {
                length: text(length),
                width: text(width),
                height: text(height),
            },
            PrimitiveShape::Cylinder { diameter, height } => PrimitiveShapeRecord::Cylinder {
                diameter: text(diameter),
                height: text(height),
            },
            PrimitiveShape::Sphere { diameter } => PrimitiveShapeRecord::Sphere {
                diameter: text(diameter),
            },
            PrimitiveShape::Torus { diameter, tube } => PrimitiveShapeRecord::Torus {
                diameter: text(diameter),
                tube: text(tube),
            },
            PrimitiveShape::Cone {
                bottom,
                top,
                height,
            } => PrimitiveShapeRecord::Cone {
                bottom: text(bottom),
                top: text(top),
                height: text(height),
            },
            PrimitiveShape::Wedge {
                length,
                width,
                height,
                top,
            } => PrimitiveShapeRecord::Wedge {
                length: text(length),
                width: text(width),
                height: text(height),
                top: text(top),
            },
            PrimitiveShape::Prism {
                sides,
                diameter,
                height,
            } => PrimitiveShapeRecord::Prism {
                sides: text(sides),
                diameter: text(diameter),
                height: text(height),
            },
        },
        plane: Lenient::Read(plane_reference_record(&primitive.plane)),
        at: primitive.at.each_ref().map(text),
        anchor: match primitive.anchor {
            PrimitiveAnchor::Corner => PrimitiveAnchorRecord::Corner,
            PrimitiveAnchor::BaseCentre => PrimitiveAnchorRecord::BaseCentre,
            PrimitiveAnchor::Centre => PrimitiveAnchorRecord::Centre,
        },
        reversed: primitive.reversed,
        operation: operation_record(primitive.operation),
    }
}

fn restore_primitive(
    record: &PrimitiveRecord,
    feature: &str,
    issues: &mut Vec<String>,
) -> Primitive {
    let mut size = |text: &str, what: &str| restore_value(text, what, "10 mm", feature, issues);
    let shape = match &record.shape {
        PrimitiveShapeRecord::Box {
            length,
            width,
            height,
        } => PrimitiveShape::Box {
            length: size(length, "length"),
            width: size(width, "width"),
            height: size(height, "height"),
        },
        PrimitiveShapeRecord::Cylinder { diameter, height } => PrimitiveShape::Cylinder {
            diameter: size(diameter, "diameter"),
            height: size(height, "height"),
        },
        PrimitiveShapeRecord::Sphere { diameter } => PrimitiveShape::Sphere {
            diameter: size(diameter, "diameter"),
        },
        PrimitiveShapeRecord::Torus { diameter, tube } => PrimitiveShape::Torus {
            diameter: size(diameter, "diameter"),
            tube: restore_value(tube, "tube diameter", "2 mm", feature, issues),
        },
        PrimitiveShapeRecord::Cone {
            bottom,
            top,
            height,
        } => PrimitiveShape::Cone {
            bottom: size(bottom, "bottom diameter"),
            top: restore_value(top, "top diameter", "0 mm", feature, issues),
            height: restore_value(height, "height", "10 mm", feature, issues),
        },
        PrimitiveShapeRecord::Wedge {
            length,
            width,
            height,
            top,
        } => PrimitiveShape::Wedge {
            length: size(length, "length"),
            width: size(width, "width"),
            height: size(height, "height"),
            top: restore_value(top, "top length", "0 mm", feature, issues),
        },
        PrimitiveShapeRecord::Prism {
            sides,
            diameter,
            height,
        } => PrimitiveShape::Prism {
            sides: restore_value(sides, "number of sides", "6", feature, issues),
            diameter: restore_value(diameter, "diameter", "10 mm", feature, issues),
            height: restore_value(height, "height", "10 mm", feature, issues),
        },
    };
    let plane = match &record.plane {
        Lenient::Read(plane) => restore_plane_reference(plane),
        Lenient::Unreadable(_) => None,
    };
    let plane = plane.unwrap_or_else(|| {
        issues.push(format!(
            "The plane or face “{feature}” stands on could not be read, so it stands on the XY \
             plane."
        ));
        PlaneReference::Principal(PrincipalPlane::Xy)
    });
    let [along, across] = &record.at;
    Primitive {
        shape,
        plane,
        at: [
            restore_value(along, "position along X", "0 mm", feature, issues),
            restore_value(across, "position along Y", "0 mm", feature, issues),
        ],
        anchor: match record.anchor {
            PrimitiveAnchorRecord::Corner => PrimitiveAnchor::Corner,
            PrimitiveAnchorRecord::BaseCentre => PrimitiveAnchor::BaseCentre,
            PrimitiveAnchorRecord::Centre => PrimitiveAnchor::Centre,
        },
        reversed: record.reversed,
        operation: restore_operation(record.operation),
    }
}

fn restore_scale(record: &ScaleRecord, feature: &str, issues: &mut Vec<String>) -> Scale {
    Scale {
        body: FeatureId::from_raw(record.body),
        factor: restore_value(&record.factor, "scale factor", "1", feature, issues),
        center: record
            .center
            .each_ref()
            .map(|text| restore_value(text, "centre", "0 mm", feature, issues)),
        frame: None,
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

fn restore_offset_face(
    record: &OffsetFaceRecord,
    feature: &str,
    issues: &mut Vec<String>,
) -> OffsetFace {
    let distance = restore_value(&record.distance, "distance", "1 mm", feature, issues);
    let faces: Vec<FaceReference> = record
        .faces
        .iter()
        .filter_map(|face| match face {
            Lenient::Read(face) => {
                restore_face(&face.face, face.origin, face.copy, &face.neighbours)
            }
            Lenient::Unreadable(_) => None,
        })
        .collect();
    if faces.len() < record.faces.len() {
        issues.push(format!(
            "Some faces moved by “{feature}” could not be read and were left where they are."
        ));
    }
    OffsetFace {
        body: FeatureId::from_raw(record.body),
        faces,
        distance,
        tangent: record.tangent,
    }
}

fn thread_record(thread: &Thread) -> ThreadRecord {
    ThreadRecord {
        body: thread.body.raw(),
        face: face_record(&thread.face),
        standard: thread.size.family().id().to_owned(),
        size: thread.size.id(),
        class: thread.class.id().to_owned(),
        left_handed: thread.hand == ThreadHand::Left,
        depth: match &thread.length {
            ThreadLength::Full => None,
            ThreadLength::Depth(depth) => Some(depth.to_stored_text()),
        },
        reversed: thread.reversed,
    }
}

fn restore_thread(record: &ThreadRecord, feature: &str, issues: &mut Vec<String>) -> Thread {
    let face = restore_face(
        &record.face.face,
        record.face.origin,
        record.face.copy,
        &record.face.neighbours,
    )
    .unwrap_or_else(|| {
        issues.push(format!(
            "The face threaded by “{feature}” could not be read; choose it again."
        ));
        FaceReference::new(FaceName::from_digest(0), None, [])
    });
    let family = ThreadFamily::from_id(&record.standard).unwrap_or_else(|| {
        issues.push(format!(
            "The thread standard of “{feature}” could not be read, so it was set to ISO metric \
             coarse."
        ));
        ThreadFamily::MetricCoarse
    });
    let size = ThreadSize::from_id(family, &record.size).unwrap_or_else(|| {
        let fallback = family.nearest(DEFAULT_THREAD_DIAMETER, ThreadSide::External);
        issues.push(format!(
            "The thread size of “{feature}” could not be read, so it was set to {}.",
            fallback.label()
        ));
        fallback
    });
    let class = family.class_from_id(&record.class).unwrap_or_else(|| {
        let fallback = family.default_class(ThreadSide::Internal);
        issues.push(format!(
            "The thread class of “{feature}” could not be read, so it was set to {}.",
            fallback.label()
        ));
        fallback
    });
    let length = match &record.depth {
        None => ThreadLength::Full,
        Some(depth) => ThreadLength::Depth(restore_value(
            depth,
            "thread depth",
            "10 mm",
            feature,
            issues,
        )),
    };
    Thread {
        body: FeatureId::from_raw(record.body),
        face,
        size,
        class,
        hand: if record.left_handed {
            ThreadHand::Left
        } else {
            ThreadHand::Right
        },
        length,
        reversed: record.reversed,
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

pub(crate) fn restore_edge(record: &EdgeRecord) -> Option<EdgeReference> {
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

pub(crate) fn restore_face(
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
    let id = ConstraintId::from_raw(record.id);
    if let Err(error) = sketch.insert_constraint(id, constraint) {
        issues.push(format!(
            "In “{feature}”, a constraint was left out because {error}."
        ));
    } else {
        if record.inactive
            && let Err(error) = sketch.set_active(id, false)
        {
            issues.push(format!(
                "In “{feature}”, a constraint was kept active because {error}."
            ));
        }
        if let Some(label) = record.label
            && let Err(error) = sketch.set_label_offset(id, Some(Vector2::from_array(label)))
        {
            issues.push(format!(
                "In “{feature}”, a dimension's label went back to its usual place because {error}."
            ));
        }
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
            diameter,
        } => {
            let (from, to) = (entity(*from), entity(*to));
            let value = value(text, DrawnValue::Distance { from, to })?;
            if *diameter {
                Constraint::AxisDiameter {
                    point: from,
                    axis: to,
                    value: diameter_of_radius(value),
                }
            } else {
                Constraint::Distance { from, to, value }
            }
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
            curve: entity(*line),
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
        ConstraintKindRecord::ArcLength { arc, value: text } => {
            let arc = entity(*arc);
            let value = value(text, DrawnValue::ArcLength(arc))?;
            Constraint::ArcLength { arc, value }
        }
        ConstraintKindRecord::Sweep { arc, value: text } => {
            let arc = entity(*arc);
            let value = value(text, DrawnValue::Sweep(arc))?;
            Constraint::Sweep { arc, value }
        }
        ConstraintKindRecord::Curvature(ids) => {
            let (a, b) = pair(*ids);
            Constraint::Curvature(a, b)
        }
    })
}

const DIAMETER_PER_RADIUS: f64 = 2.0;

fn radius_of_diameter(diameter: &Expression) -> Expression {
    match *diameter {
        Expression::Measure(value, unit) => Expression::Measure(value / DIAMETER_PER_RADIUS, unit),
        Expression::Number(value) => Expression::Number(value / DIAMETER_PER_RADIUS),
        _ => Expression::binary(
            BinaryOperator::Divide,
            diameter.clone(),
            Expression::Number(DIAMETER_PER_RADIUS),
        ),
    }
}

fn diameter_of_radius(radius: Expression) -> Expression {
    match radius {
        Expression::Measure(value, unit) => Expression::Measure(value * DIAMETER_PER_RADIUS, unit),
        Expression::Number(value) => Expression::Number(value * DIAMETER_PER_RADIUS),
        Expression::Binary(BinaryOperator::Divide, diameter, divisor)
            if *divisor == Expression::Number(DIAMETER_PER_RADIUS) =>
        {
            *diameter
        }
        radius => Expression::binary(
            BinaryOperator::Multiply,
            radius,
            Expression::Number(DIAMETER_PER_RADIUS),
        ),
    }
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
    ArcLength(EntityId),
    Sweep(EntityId),
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
            Self::ArcLength(_) => "an arc length",
            Self::Sweep(_) => "a sweep",
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
            Self::ArcLength(_) => "drawn arc length",
            Self::Sweep(_) => "drawn sweep",
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
            Self::ArcLength(arc) => Constraint::ArcLength { arc, value },
            Self::Sweep(arc) => Constraint::Sweep { arc, value },
        };
        let measured = sketch.measured(&constraint)?;
        let quantity = match self {
            Self::Angle { .. } | Self::Sweep(_) => Quantity::angle(measured),
            Self::Radius(_) | Self::Diameter(_) | Self::ArcLength(_) if measured <= 0.0 => {
                return None;
            }
            Self::Distance { .. }
            | Self::HorizontalDistance { .. }
            | Self::VerticalDistance { .. }
            | Self::Radius(_)
            | Self::Diameter(_)
            | Self::ArcLength(_) => Quantity::length(measured),
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
