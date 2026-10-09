use caditor_document::{
    BlendKind, CombineOperation, Datum, Document, FeatureKind, PatternKind, PrimitiveKind,
    PrincipalGeometry, SolidFeature,
};
use egui_phosphor::regular as phosphor;

use crate::{
    analysis::AnalysisCommand,
    commands::{CameraMove, Command},
    editing::Tool,
    icon_font,
    selection::Pickable,
    shape_modes::{BlendMode, CircleMode, PolygonMode, RectangleMode, ShapeMode, SlotMode},
    sketch_tools::ConstraintTool,
};

pub const SEARCH: &str = phosphor::MAGNIFYING_GLASS;
pub const EDIT: &str = phosphor::PENCIL_SIMPLE;
pub const DONE: &str = phosphor::CHECK;
pub const MORE: &str = phosphor::DOTS_THREE;
pub const DELETE: &str = phosphor::TRASH;
pub const REMOVE_BODY: &str = phosphor::CUBE_FOCUS;
pub const COPY_BODY: &str = phosphor::COPY;
pub const REMOVE: &str = phosphor::X;
pub const CLOSE: &str = phosphor::X;
pub const ADD: &str = phosphor::PLUS;
pub const FEATURE_GROUP: &str = phosphor::FOLDER_SIMPLE;
pub const SUBTRACT: &str = phosphor::MINUS;
pub const EXPANDED: &str = phosphor::CARET_DOWN;
pub const COLLAPSED: &str = phosphor::CARET_RIGHT;
pub const FAILED: &str = phosphor::WARNING_CIRCLE;
pub const OUTDATED: &str = phosphor::PAUSE_CIRCLE;
pub const PENDING: &str = phosphor::CIRCLE_NOTCH;
pub const UP_TO_DATE: &str = phosphor::CHECK_CIRCLE;
pub const INFO: &str = phosphor::INFO;
pub const WARNING: &str = phosphor::WARNING;
pub const UPDATE_REFERENCES: &str = phosphor::LINK_SIMPLE;
pub const TIP: &str = phosphor::LIGHTBULB;
pub const SELECTION: &str = phosphor::SELECTION;
pub const UNIT: &str = phosphor::RULER;
pub const RECENT: &str = phosphor::CLOCK;
pub const KEPT_VERSION: &str = phosphor::BOOKMARK_SIMPLE;
pub const RECOVER: &str = phosphor::LIFEBUOY;
pub const SAMPLE: &str = phosphor::CUBE;
pub const CHOOSE_IN_VIEW: &str = phosphor::CURSOR_CLICK;
pub const USE_SELECTED: &str = phosphor::ARROW_SQUARE_IN;
pub const HIDE: &str = phosphor::EYE_SLASH;
pub const SHOW: &str = phosphor::EYE;
pub const FILE: &str = phosphor::FILE;
pub const DROP_FILES: &str = phosphor::DOWNLOAD_SIMPLE;
pub const ATTACHED: &str = phosphor::PUSH_PIN;
pub const RENAME: &str = phosphor::TEXTBOX;
pub const MOVE_UP: &str = phosphor::ARROW_UP;
pub const NOTE: &str = phosphor::NOTE;
pub const EDIT_NOTE: &str = phosphor::NOTE_PENCIL;
pub const MOVE_DOWN: &str = phosphor::ARROW_DOWN;
pub const SUPPRESS: &str = phosphor::PROHIBIT_INSET;
pub const UNSUPPRESS: &str = phosphor::POWER;
pub const ROLL_TO_HERE: &str = phosphor::REWIND;
pub const ROLL_TO_END: &str = phosphor::SKIP_FORWARD;
pub const ROLLBACK_UP: &str = phosphor::CARET_DOUBLE_UP;
pub const ROLLBACK_DOWN: &str = phosphor::CARET_DOUBLE_DOWN;
pub const ROLLBACK_BAR: &str = phosphor::DOTS_SIX;
pub const ROLLED_BACK: &str = phosphor::CLOCK_COUNTER_CLOCKWISE;
pub const PRINCIPAL_GROUP: &str = PLANE;
pub const BODIES: &str = phosphor::CUBE;
pub const BODY_APPEARANCE: &str = phosphor::PAINT_BUCKET;
pub const CONSTRUCTION: &str = phosphor::CIRCLE_DASHED;
pub const MINIMIZE: &str = phosphor::MINUS;
pub const MAXIMIZE: &str = phosphor::SQUARE;
pub const RESTORE: &str = phosphor::COPY;
pub const FULL_SCREEN: &str = phosphor::CORNERS_OUT;
pub const LEAVE_FULL_SCREEN: &str = phosphor::CORNERS_IN;
pub const COPY_PATH: &str = phosphor::CLIPBOARD_TEXT;
pub const BREADCRUMB: &str = phosphor::CARET_RIGHT;
pub const GENERAL: &str = phosphor::SLIDERS_HORIZONTAL;
pub const APPEARANCE: &str = phosphor::PALETTE;
pub const NAVIGATION: &str = phosphor::COMPASS;
pub const GRAPHICS: &str = phosphor::MONITOR;
pub const MEASURE: &str = phosphor::RULER;
pub const INTERFERENCE: &str = phosphor::INTERSECT_SQUARE;
pub const ANALYSIS: &str = phosphor::GAUGE;
pub const CURVATURE_COMB: &str = phosphor::CHART_LINE;
pub const COPY: &str = phosphor::COPY_SIMPLE;
pub const TEMPLATE: &str = phosphor::STAMP;
pub const FEATURES: &str = phosphor::TREE_STRUCTURE;
pub const PARAMETERS: &str = phosphor::FUNCTION;
pub const GO_TO: &str = phosphor::ARROW_RIGHT;
pub const SHOW_PLACE: &str = phosphor::MAP_PIN;
pub const DETACH: &str = phosphor::LINK_BREAK;
pub const UNUSED: &str = phosphor::LINK_SIMPLE_BREAK;
pub const UNDER_POINTER: &str = phosphor::STACK;
const FACE: &str = phosphor::SQUARE_HALF;
const EDGE: &str = phosphor::LINE_SEGMENT;
const VERTEX: &str = phosphor::DOT_OUTLINE;
const REGION: &str = phosphor::SELECTION;
const CENTRE_OF_MASS: &str = phosphor::TARGET;
const ORIGIN: &str = phosphor::CROSSHAIR;

pub fn command(command: Command) -> &'static str {
    match command {
        Command::Palette => SEARCH,
        Command::New => phosphor::FILE_PLUS,
        Command::NewFromTemplate => TEMPLATE,
        Command::SaveAsTemplate => phosphor::BOOKMARK_SIMPLE,
        Command::Open => phosphor::FOLDER_OPEN,
        Command::Save => phosphor::FLOPPY_DISK,
        Command::SaveAs => phosphor::FLOPPY_DISK_BACK,
        Command::VersionHistory => phosphor::CLOCK_COUNTER_CLOCKWISE,
        Command::ModelProperties => phosphor::IDENTIFICATION_CARD,
        Command::Import => phosphor::DOWNLOAD_SIMPLE,
        Command::Export => phosphor::EXPORT,
        Command::ExportImage => phosphor::IMAGE,
        Command::ExportSketch => phosphor::EXPORT,
        Command::ExportFace => phosphor::EXPORT,
        Command::KeepDrawingConstruction => CONSTRUCTION,
        Command::Preferences => phosphor::GEAR,
        Command::KeyboardShortcuts => phosphor::KEYBOARD,
        Command::Quit => phosphor::SIGN_OUT,
        Command::Undo => phosphor::ARROW_U_UP_LEFT,
        Command::UndoHistory => phosphor::CLOCK_COUNTER_CLOCKWISE,
        Command::Redo => phosphor::ARROW_U_UP_RIGHT,
        Command::NewSketch => phosphor::PENCIL_LINE,
        Command::FinishSketch => phosphor::CHECK,
        Command::ReverseArc => phosphor::ARROWS_COUNTER_CLOCKWISE,
        Command::MoreSides => ADD,
        Command::FewerSides => SUBTRACT,
        Command::ToggleConstraintActive => phosphor::PROHIBIT,
        Command::SplitCurve => phosphor::GIT_COMMIT,
        Command::BreakCurves => phosphor::GIT_FORK,
        Command::RotateGeometry => phosphor::ARROW_ARC_LEFT,
        Command::ScaleGeometry => phosphor::RESIZE,
        Command::Construction => CONSTRUCTION,
        Command::MoveGeometry => phosphor::ARROWS_OUT_CARDINAL,
        Command::SelectAll => phosphor::SELECTION_ALL,
        Command::SelectFree => phosphor::LOCK_SIMPLE_OPEN,
        Command::CopyGeometry => COPY,
        Command::CutGeometry => phosphor::SCISSORS,
        Command::PasteGeometry => phosphor::CLIPBOARD,
        Command::IntersectBody => phosphor::INTERSECTION,
        Command::SketchTool(tool) => self::tool(tool),
        Command::ShapeMode(mode) => shape_mode(mode),
        Command::Constraint(tool) => constraint(tool),
        Command::DeleteSelection => DELETE,
        Command::Extrude => EXTRUDE,
        Command::Revolve => REVOLVE,
        Command::Fillet => blend(BlendKind::Fillet),
        Command::Chamfer => blend(BlendKind::Chamfer),
        Command::Shell => SHELL,
        Command::OffsetFace => OFFSET_FACE,
        Command::NewBox => primitive(PrimitiveKind::Box),
        Command::NewCylinder => primitive(PrimitiveKind::Cylinder),
        Command::NewSphere => primitive(PrimitiveKind::Sphere),
        Command::NewTorus => primitive(PrimitiveKind::Torus),
        Command::Thread => THREAD,
        Command::NewCone => primitive(PrimitiveKind::Cone),
        Command::NewWedge => primitive(PrimitiveKind::Wedge),
        Command::NewPrism => primitive(PrimitiveKind::Prism),
        Command::Combine => COMBINE,
        Command::Move => MOVE,
        Command::CopyBody => COPY_BODY,
        Command::Mirror => MIRROR,
        Command::Split => SPLIT,
        Command::Mate => MATE,
        Command::Scale => SCALE,
        Command::RenameBody => EDIT,
        Command::RemoveBody => REMOVE_BODY,
        Command::SelectBody => BODIES,
        Command::SaveSelectionSet => phosphor::SELECTION_PLUS,
        Command::SelectionSets => phosphor::LIST_CHECKS,
        Command::BodyAppearance => BODY_APPEARANCE,
        Command::Hole => HOLE,
        Command::LinearPattern => LINEAR_PATTERN,
        Command::CircularPattern => CIRCULAR_PATTERN,
        Command::DatumPlane => PLANE,
        Command::DatumAxis => AXIS,
        Command::DatumPoint => POINT,
        Command::CoordinateSystem => COORDINATE_SYSTEM,
        Command::ScaleModel => phosphor::ARROWS_OUT,
        Command::FitView => phosphor::FRAME_CORNERS,
        Command::Measure => MEASURE,
        Command::Interference => INTERFERENCE,
        Command::Analysis(AnalysisCommand::Comb) => CURVATURE_COMB,
        Command::Analysis(_) => ANALYSIS,
        Command::ToggleProjection => phosphor::PERSPECTIVE,
        Command::AutomaticProjection => phosphor::PERSPECTIVE,
        Command::ToggleSnapping => phosphor::MAGNET,
        Command::ToggleGridSnapping => phosphor::GRID_FOUR,
        Command::ToggleLasso => phosphor::LASSO,
        Command::TogglePaintSelection => phosphor::PAINT_BRUSH,
        Command::ToggleSelectThrough => phosphor::SELECTION_BACKGROUND,
        Command::CycleSelectionPriority => phosphor::FUNNEL,
        Command::ToggleTypedDimensions => phosphor::RULER,
        Command::ToggleFirstDimensionScales => phosphor::RESIZE,
        Command::ToggleGlyphs => phosphor::SHAPES,
        Command::ToggleCentresOfMass => phosphor::TARGET,
        Command::MinimizeWindow => MINIMIZE,
        Command::MaximizeWindow => MAXIMIZE,
        Command::FullScreen => FULL_SCREEN,
        Command::LargerInterface => phosphor::MAGNIFYING_GLASS_PLUS,
        Command::SmallerInterface => phosphor::MAGNIFYING_GLASS_MINUS,
        Command::NormalInterface => phosphor::TEXT_AA,
        Command::View(_) => phosphor::CUBE_FOCUS,
        Command::Filter(_) => phosphor::FUNNEL,
        Command::Style(_) => phosphor::CUBE_TRANSPARENT,
        Command::Camera(camera) => camera_move(camera),
        Command::HighlightNext => phosphor::ARROW_RIGHT,
        Command::HighlightPrevious => phosphor::ARROW_LEFT,
        Command::ActivateHighlighted => phosphor::CURSOR_CLICK,
        Command::ListUnderPointer => UNDER_POINTER,
        Command::HideSelection | Command::ToggleVisibility => HIDE,
        Command::HideOthers => phosphor::EYE_CLOSED,
        Command::LookAtFace => phosphor::CUBE_FOCUS,
        Command::LookAtSketch => phosphor::SCAN,
        Command::SelectAllShapes => phosphor::SELECTION_ALL,
        Command::SelectTangentEdges => phosphor::LINE_SEGMENTS,
        Command::SelectTangentFaces => phosphor::CYLINDER,
        Command::SelectHole => HOLE,
        Command::SelectFaceEdges => phosphor::POLYGON,
        Command::ShowAll => SHOW,
        Command::TogglePrincipal => PRINCIPAL_GROUP,
        Command::ToggleSketches => SKETCH,
        Command::ToggleDatums => AXIS,
        Command::ToggleBodies => BODIES,
        Command::SaveView => phosphor::CAMERA_PLUS,
        Command::SavedViews => phosphor::BOOKMARKS,
        Command::SetHomeView => phosphor::HOUSE_LINE,
        Command::ResetHomeView => phosphor::ARROW_COUNTER_CLOCKWISE,
        Command::OpenSample(_) => SAMPLE,
        Command::OpenRecent(_) => RECENT,
        Command::ClearRecent => DELETE,
        Command::RecoverUnsaved => RECOVER,
        Command::CancelExport
        | Command::CancelImageExport
        | Command::CancelImport
        | Command::CancelRecompute => phosphor::STOP_CIRCLE,
        Command::Recompute => phosphor::ARROW_CLOCKWISE,
        Command::RenameFeature | Command::RenameGroup => RENAME,
        Command::GroupFeatures => FEATURE_GROUP,
        Command::Ungroup => phosphor::FOLDER_MINUS,
        Command::MoveFeatureUp => MOVE_UP,
        Command::MoveFeatureDown => MOVE_DOWN,
        Command::DeleteFeature => DELETE,
        Command::CopyFeatures => COPY,
        Command::PasteFeatures => phosphor::CLIPBOARD,
        Command::SuppressFeature => SUPPRESS,
        Command::RollToHere => ROLL_TO_HERE,
        Command::RollToEnd => ROLL_TO_END,
        Command::RollbackUp => ROLLBACK_UP,
        Command::RollbackDown => ROLLBACK_DOWN,
        Command::EditFeature => EDIT,
        Command::CloseFeature => DONE,
        Command::DetachSketch => phosphor::LINK_BREAK,
        Command::ClearChosenRegions => phosphor::SELECTION_SLASH,
        Command::PlaceSketch
        | Command::UseSelectedAxis
        | Command::ExtrudeUpToSelected
        | Command::StartAtSelected
        | Command::MirrorAcrossSelected
        | Command::SplitAlongSelected
        | Command::DatumUseSelected
        | Command::DatumTurnAboutSelected
        | Command::PatternUseSelected
        | Command::PatternSecondUseSelected
        | Command::MoveTurnAboutSelected => USE_SELECTED,
        Command::FilterFeatures => SEARCH,
        Command::AddParameter => ADD,
        Command::DeleteParameter => DELETE,
        Command::MoveParameterUp => MOVE_UP,
        Command::MoveParameterDown => MOVE_DOWN,
        Command::ParameterNote => EDIT_NOTE,
        Command::ShowFirstFailed => FAILED,
        Command::UpdateReferences => UPDATE_REFERENCES,
        Command::ReplaceImport => phosphor::FILE_ARROW_UP,
        Command::ReloadImport => phosphor::ARROW_CLOCKWISE,
        Command::DismissNotice | Command::DismissTip => CLOSE,
        Command::HideTips => phosphor::EYE_SLASH,
        Command::Welcome => phosphor::HAND_WAVING,
        Command::About => phosphor::INFO,
        Command::Messages => phosphor::CHAT_TEXT,
    }
}

const EXTRUDE: &str = icon_font::EXTRUDE;
const REVOLVE: &str = icon_font::REVOLVE;
const SHELL: &str = icon_font::SHELL;
const COMBINE: &str = phosphor::UNITE;
const MOVE: &str = phosphor::HAND_GRABBING;
const MIRROR: &str = phosphor::FLIP_HORIZONTAL;
const SPLIT: &str = phosphor::SQUARE_SPLIT_HORIZONTAL;
const MATE: &str = phosphor::MAGNET;
const OFFSET_FACE: &str = phosphor::ARROWS_OUT_LINE_VERTICAL;
const THREAD: &str = phosphor::SPIRAL;
const SCALE: &str = phosphor::RESIZE;
const HOLE: &str = phosphor::CIRCLE_DASHED;
const LINEAR_PATTERN: &str = icon_font::LINEAR_PATTERN;
const CIRCULAR_PATTERN: &str = icon_font::CIRCULAR_PATTERN;
const PLANE: &str = phosphor::PARALLELOGRAM;
const AXIS: &str = phosphor::ARROW_LINE_UP_RIGHT;
const POINT: &str = phosphor::CROSSHAIR_SIMPLE;
const COORDINATE_SYSTEM: &str = phosphor::VECTOR_THREE;
pub const SKETCH: &str = phosphor::PENCIL_LINE;
const IMPORTED: &str = phosphor::CUBE;

pub fn tool(tool: Tool) -> &'static str {
    match tool {
        Tool::Select => phosphor::CURSOR,
        Tool::Point => phosphor::DOT_OUTLINE,
        Tool::Line => phosphor::LINE_SEGMENT,
        Tool::Rectangle => phosphor::RECTANGLE,
        Tool::Circle => phosphor::CIRCLE,
        Tool::Arc => phosphor::CIRCLE_HALF,
        Tool::ThreePointArc => phosphor::RAINBOW,
        Tool::TangentArc => phosphor::ARROW_BEND_UP_RIGHT,
        Tool::Ellipse => phosphor::EGG,
        Tool::EllipticalArc => phosphor::EGG_CRACK,
        Tool::Slot => phosphor::PILL,
        Tool::Polygon => phosphor::HEXAGON,
        Tool::Spline => phosphor::BEZIER_CURVE,
        Tool::Trim => phosphor::SCISSORS,
        Tool::Extend => phosphor::ARROW_LINE_RIGHT,
        Tool::Offset => phosphor::WAVES,
        Tool::Mirror => phosphor::SQUARE_SPLIT_HORIZONTAL,
        Tool::RectangularPattern => LINEAR_PATTERN,
        Tool::CircularPattern => CIRCULAR_PATTERN,
        Tool::TangentCircle => phosphor::CIRCLES_THREE,
        Tool::Fillet => blend(BlendKind::Fillet),
        Tool::Chamfer => blend(BlendKind::Chamfer),
        Tool::Project => phosphor::ARROW_FAT_LINES_DOWN,
        Tool::Intersect => phosphor::INTERSECTION,
        Tool::Dimension => phosphor::MAGIC_WAND,
        Tool::BlendCurve => phosphor::PATH,
    }
}

pub fn shape_mode(mode: ShapeMode) -> &'static str {
    match mode {
        ShapeMode::Rectangle(RectangleMode::Corners) => tool(Tool::Rectangle),
        ShapeMode::Rectangle(RectangleMode::Center) => phosphor::ARROWS_OUT,
        ShapeMode::Rectangle(RectangleMode::ThreePoints) => phosphor::DIAMOND,
        ShapeMode::Circle(CircleMode::Center) => tool(Tool::Circle),
        ShapeMode::Circle(CircleMode::TwoPoints) => phosphor::PROHIBIT,
        ShapeMode::Circle(CircleMode::ThreePoints) => phosphor::DOTS_THREE_CIRCLE,
        ShapeMode::Polygon(PolygonMode::Corner) => tool(Tool::Polygon),
        ShapeMode::Polygon(PolygonMode::SideMiddle) => phosphor::OCTAGON,
        ShapeMode::Polygon(PolygonMode::Side) => phosphor::PENTAGON,
        ShapeMode::Slot(SlotMode::Ends) => tool(Tool::Slot),
        ShapeMode::Slot(SlotMode::Center) => phosphor::ARROWS_LEFT_RIGHT,
        ShapeMode::Slot(SlotMode::Arc) => phosphor::MAGNET,
        ShapeMode::Blend(BlendMode::Tangent) => tool(Tool::BlendCurve),
        ShapeMode::Blend(BlendMode::Curvature) => phosphor::WAVE_SINE,
    }
}

pub fn constraint(tool: ConstraintTool) -> &'static str {
    match tool {
        ConstraintTool::Coincident => phosphor::CROSSHAIR_SIMPLE,
        ConstraintTool::Midpoint => phosphor::GIT_COMMIT,
        ConstraintTool::Concentric => phosphor::TARGET,
        ConstraintTool::Collinear => phosphor::DOTS_THREE_OUTLINE,
        ConstraintTool::Fix => phosphor::LOCK_SIMPLE,
        ConstraintTool::Horizontal => phosphor::ARROWS_OUT_LINE_HORIZONTAL,
        ConstraintTool::Vertical => phosphor::ARROWS_OUT_LINE_VERTICAL,
        ConstraintTool::Parallel => phosphor::PAUSE,
        ConstraintTool::Perpendicular => phosphor::ANGLE,
        ConstraintTool::Tangent => phosphor::ARROW_ARC_RIGHT,
        ConstraintTool::Curvature => phosphor::WAVE_SINE,
        ConstraintTool::Equal => phosphor::EQUALS,
        ConstraintTool::Symmetric => phosphor::FLIP_HORIZONTAL,
        ConstraintTool::Distance => phosphor::RULER,
        ConstraintTool::HorizontalDistance => phosphor::ARROWS_HORIZONTAL,
        ConstraintTool::VerticalDistance => phosphor::ARROWS_VERTICAL,
        ConstraintTool::Angle => phosphor::COMPASS_TOOL,
        ConstraintTool::Radius => phosphor::CIRCLE_DASHED,
        ConstraintTool::Diameter => phosphor::PROHIBIT,
    }
}

pub fn blend(kind: BlendKind) -> &'static str {
    match kind {
        BlendKind::Fillet => icon_font::FILLET,
        BlendKind::Chamfer => icon_font::CHAMFER,
    }
}

fn camera_move(camera: CameraMove) -> &'static str {
    match camera {
        CameraMove::OrbitLeft => phosphor::ARROW_ARC_LEFT,
        CameraMove::OrbitRight => phosphor::ARROW_ARC_RIGHT,
        CameraMove::OrbitUp => phosphor::ARROW_BEND_RIGHT_UP,
        CameraMove::OrbitDown => phosphor::ARROW_BEND_RIGHT_DOWN,
        CameraMove::PanLeft | CameraMove::PanRight | CameraMove::PanUp | CameraMove::PanDown => {
            phosphor::HAND_GRABBING
        }
        CameraMove::ZoomIn => phosphor::MAGNIFYING_GLASS_PLUS,
        CameraMove::ZoomOut => phosphor::MAGNIFYING_GLASS_MINUS,
    }
}

pub fn principal(geometry: PrincipalGeometry) -> &'static str {
    match geometry {
        PrincipalGeometry::Origin => ORIGIN,
        PrincipalGeometry::Axis(_) => AXIS,
        PrincipalGeometry::Plane(_) => PLANE,
    }
}

pub fn combine(operation: CombineOperation) -> &'static str {
    match operation {
        CombineOperation::Join => phosphor::UNITE,
        CombineOperation::Cut => phosphor::SUBTRACT,
        CombineOperation::Intersect => phosphor::INTERSECT,
    }
}

pub fn primitive(kind: PrimitiveKind) -> &'static str {
    match kind {
        PrimitiveKind::Box => phosphor::CUBE,
        PrimitiveKind::Cylinder => phosphor::CYLINDER,
        PrimitiveKind::Sphere => phosphor::SPHERE,
        PrimitiveKind::Torus => phosphor::DISC,
        PrimitiveKind::Cone => phosphor::TRAFFIC_CONE,
        PrimitiveKind::Wedge => phosphor::TRIANGLE,
        PrimitiveKind::Prism => phosphor::HEXAGON,
    }
}

pub fn feature(kind: &FeatureKind) -> &'static str {
    match kind {
        FeatureKind::Sketch(_) => SKETCH,
        FeatureKind::Solid(SolidFeature::Extrude(_)) => EXTRUDE,
        FeatureKind::Solid(SolidFeature::Revolve(_)) => REVOLVE,
        FeatureKind::Blend(blend) => self::blend(blend.kind),
        FeatureKind::Shell(_) => SHELL,
        FeatureKind::OffsetFace(_) => OFFSET_FACE,
        FeatureKind::Primitive(primitive) => self::primitive(primitive.shape.kind()),
        FeatureKind::Thread(_) => THREAD,
        FeatureKind::Combine(combine) => self::combine(combine.operation),
        FeatureKind::Move(movement) if movement.copy => COPY_BODY,
        FeatureKind::Move(_) => MOVE,
        FeatureKind::Mirror(_) => MIRROR,
        FeatureKind::Split(_) => SPLIT,
        FeatureKind::Mate(_) => MATE,
        FeatureKind::Scale(_) => SCALE,
        FeatureKind::Hole(_) => HOLE,
        FeatureKind::Pattern(pattern) => match pattern.kind {
            PatternKind::Linear { .. } => LINEAR_PATTERN,
            PatternKind::Circular(_) => CIRCULAR_PATTERN,
        },
        FeatureKind::Datum(Datum::Plane(_) | Datum::PlaneThrough(_)) => PLANE,
        FeatureKind::Datum(Datum::Axis(_)) => AXIS,
        FeatureKind::Datum(Datum::Point(_) | Datum::PointBy(_)) => POINT,
        FeatureKind::Datum(Datum::Frame(_)) => COORDINATE_SYSTEM,
        FeatureKind::Import(_) => IMPORTED,
        FeatureKind::Remove(_) => REMOVE_BODY,
    }
}

pub fn pickable(pickable: Pickable, document: &Document) -> &'static str {
    match pickable {
        Pickable::Origin => ORIGIN,
        Pickable::Axis(_) | Pickable::FrameAxis { .. } => AXIS,
        Pickable::Plane(_) | Pickable::FramePlane { .. } => PLANE,
        Pickable::SketchEntity { .. } => SKETCH,
        Pickable::SketchConstraint { .. } => MEASURE,
        Pickable::SketchRegion { .. } | Pickable::Region { .. } => REGION,
        Pickable::Face { .. } | Pickable::ShellFace { .. } => FACE,
        Pickable::Edge { .. } | Pickable::BlendEdge { .. } => EDGE,
        Pickable::Vertex { .. } => VERTEX,
        Pickable::Datum(datum) => document
            .feature(datum)
            .map_or(PLANE, |feature| self::feature(&feature.kind)),
        Pickable::CentreOfMass(_) => CENTRE_OF_MASS,
    }
}
