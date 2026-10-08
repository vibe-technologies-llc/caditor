use caditor_document::{
    BlendKind, CombineOperation, Datum, FeatureKind, PatternKind, PrincipalGeometry, SolidFeature,
};
use egui_phosphor::regular as phosphor;

use crate::{
    commands::{CameraMove, Command},
    editing::Tool,
    shape_modes::{CircleMode, PolygonMode, RectangleMode, ShapeMode, SlotMode},
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
pub const COPY: &str = phosphor::COPY_SIMPLE;
pub const FEATURES: &str = phosphor::TREE_STRUCTURE;
pub const PARAMETERS: &str = phosphor::FUNCTION;
pub const GO_TO: &str = phosphor::ARROW_RIGHT;
pub const SHOW_PLACE: &str = phosphor::MAP_PIN;
pub const DETACH: &str = phosphor::LINK_BREAK;
pub const UNUSED: &str = phosphor::LINK_SIMPLE_BREAK;
const ORIGIN: &str = phosphor::CROSSHAIR;

pub fn command(command: Command) -> &'static str {
    match command {
        Command::Palette => SEARCH,
        Command::New => phosphor::FILE_PLUS,
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
        Command::Construction => CONSTRUCTION,
        Command::MoveGeometry => phosphor::ARROWS_OUT_CARDINAL,
        Command::SelectAll => phosphor::SELECTION_ALL,
        Command::CopyGeometry => COPY,
        Command::CutGeometry => phosphor::SCISSORS,
        Command::PasteGeometry => phosphor::CLIPBOARD,
        Command::SketchTool(tool) => self::tool(tool),
        Command::ShapeMode(mode) => shape_mode(mode),
        Command::Constraint(tool) => constraint(tool),
        Command::DeleteSelection => DELETE,
        Command::Extrude => EXTRUDE,
        Command::Revolve => REVOLVE,
        Command::Fillet => blend(BlendKind::Fillet),
        Command::Chamfer => blend(BlendKind::Chamfer),
        Command::Shell => SHELL,
        Command::Combine => COMBINE,
        Command::Move => MOVE,
        Command::CopyBody => COPY_BODY,
        Command::Mirror => MIRROR,
        Command::Split => SPLIT,
        Command::Scale => SCALE,
        Command::RenameBody => EDIT,
        Command::RemoveBody => REMOVE_BODY,
        Command::SelectBody => BODIES,
        Command::BodyAppearance => BODY_APPEARANCE,
        Command::Hole => HOLE,
        Command::LinearPattern => LINEAR_PATTERN,
        Command::CircularPattern => CIRCULAR_PATTERN,
        Command::DatumPlane => PLANE,
        Command::DatumAxis => AXIS,
        Command::DatumPoint => POINT,
        Command::FitView => phosphor::FRAME_CORNERS,
        Command::Measure => MEASURE,
        Command::Interference => INTERFERENCE,
        Command::ToggleProjection => phosphor::PERSPECTIVE,
        Command::ToggleSnapping => phosphor::MAGNET,
        Command::ToggleGridSnapping => phosphor::GRID_FOUR,
        Command::ToggleTypedDimensions => phosphor::RULER,
        Command::ToggleGlyphs => phosphor::SHAPES,
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
        Command::OpenSample(_) => SAMPLE,
        Command::OpenRecent(_) => RECENT,
        Command::ClearRecent => DELETE,
        Command::RecoverUnsaved => RECOVER,
        Command::CancelExport | Command::CancelImageExport | Command::CancelRecompute => {
            phosphor::STOP_CIRCLE
        }
        Command::Recompute => phosphor::ARROW_CLOCKWISE,
        Command::RenameFeature => RENAME,
        Command::MoveFeatureUp => MOVE_UP,
        Command::MoveFeatureDown => MOVE_DOWN,
        Command::DeleteFeature => DELETE,
        Command::SuppressFeature => SUPPRESS,
        Command::RollToHere => ROLL_TO_HERE,
        Command::RollToEnd => ROLL_TO_END,
        Command::RollbackUp => ROLLBACK_UP,
        Command::RollbackDown => ROLLBACK_DOWN,
        Command::EditFeature => EDIT,
        Command::CloseFeature => DONE,
        Command::DetachSketch => phosphor::LINK_BREAK,
        Command::PlaceSketch
        | Command::UseSelectedAxis
        | Command::ExtrudeUpToSelected
        | Command::StartAtSelected
        | Command::MirrorAcrossSelected
        | Command::SplitAlongSelected
        | Command::DatumUseSelected
        | Command::DatumTurnAboutSelected
        | Command::PatternUseSelected
        | Command::PatternSecondUseSelected => USE_SELECTED,
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

const EXTRUDE: &str = phosphor::ARROW_FAT_LINE_UP;
const REVOLVE: &str = phosphor::ARROWS_CLOCKWISE;
const SHELL: &str = phosphor::CUBE_TRANSPARENT;
const COMBINE: &str = phosphor::UNITE;
const MOVE: &str = phosphor::HAND_GRABBING;
const MIRROR: &str = phosphor::FLIP_HORIZONTAL;
const SPLIT: &str = phosphor::SQUARE_SPLIT_HORIZONTAL;
const SCALE: &str = phosphor::RESIZE;
const HOLE: &str = phosphor::CIRCLE_DASHED;
const LINEAR_PATTERN: &str = phosphor::SQUARES_FOUR;
const CIRCULAR_PATTERN: &str = phosphor::SPINNER;
const PLANE: &str = phosphor::PARALLELOGRAM;
const AXIS: &str = phosphor::ARROW_LINE_UP_RIGHT;
const POINT: &str = phosphor::CROSSHAIR_SIMPLE;
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
        Tool::Slot => phosphor::PILL,
        Tool::Polygon => phosphor::HEXAGON,
        Tool::Spline => phosphor::BEZIER_CURVE,
        Tool::Trim => phosphor::SCISSORS,
        Tool::Extend => phosphor::ARROW_LINE_RIGHT,
        Tool::Offset => phosphor::WAVES,
        Tool::Mirror => phosphor::SQUARE_SPLIT_HORIZONTAL,
        Tool::Fillet => blend(BlendKind::Fillet),
        Tool::Project => phosphor::ARROW_FAT_LINES_DOWN,
        Tool::Dimension => phosphor::MAGIC_WAND,
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
        BlendKind::Fillet => phosphor::CORNERS_OUT,
        BlendKind::Chamfer => phosphor::POLYGON,
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

pub fn feature(kind: &FeatureKind) -> &'static str {
    match kind {
        FeatureKind::Sketch(_) => SKETCH,
        FeatureKind::Solid(SolidFeature::Extrude(_)) => EXTRUDE,
        FeatureKind::Solid(SolidFeature::Revolve(_)) => REVOLVE,
        FeatureKind::Blend(blend) => self::blend(blend.kind),
        FeatureKind::Shell(_) => SHELL,
        FeatureKind::Combine(combine) => self::combine(combine.operation),
        FeatureKind::Move(movement) if movement.copy => COPY_BODY,
        FeatureKind::Move(_) => MOVE,
        FeatureKind::Mirror(_) => MIRROR,
        FeatureKind::Split(_) => SPLIT,
        FeatureKind::Scale(_) => SCALE,
        FeatureKind::Hole(_) => HOLE,
        FeatureKind::Pattern(pattern) => match pattern.kind {
            PatternKind::Linear { .. } => LINEAR_PATTERN,
            PatternKind::Circular(_) => CIRCULAR_PATTERN,
        },
        FeatureKind::Datum(Datum::Plane(_) | Datum::PlaneThrough(_)) => PLANE,
        FeatureKind::Datum(Datum::Axis(_)) => AXIS,
        FeatureKind::Datum(Datum::Point(_)) => POINT,
        FeatureKind::Import(_) => IMPORTED,
        FeatureKind::Remove(_) => REMOVE_BODY,
    }
}
