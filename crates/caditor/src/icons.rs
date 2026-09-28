use caditor_document::{BlendKind, Datum, FeatureKind, SolidFeature};
use egui_phosphor::regular as phosphor;

use crate::{
    commands::{CameraMove, Command},
    editing::Tool,
    sketch_tools::ConstraintTool,
};

pub const SEARCH: &str = phosphor::MAGNIFYING_GLASS;
pub const EDIT: &str = phosphor::PENCIL_SIMPLE;
pub const DONE: &str = phosphor::CHECK;
pub const MORE: &str = phosphor::DOTS_THREE;
pub const DELETE: &str = phosphor::TRASH;
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
pub const TIP: &str = phosphor::LIGHTBULB;
pub const SELECTION: &str = phosphor::SELECTION;
pub const UNIT: &str = phosphor::RULER;
pub const RECENT: &str = phosphor::CLOCK;
pub const RECOVER: &str = phosphor::LIFEBUOY;
pub const SAMPLE: &str = phosphor::CUBE;
pub const CHOOSE_IN_VIEW: &str = phosphor::CURSOR_CLICK;
pub const USE_SELECTED: &str = phosphor::ARROW_SQUARE_IN;
pub const FILE: &str = phosphor::FILE;
pub const ATTACHED: &str = phosphor::PUSH_PIN;
pub const RENAME: &str = phosphor::TEXTBOX;
pub const MOVE_UP: &str = phosphor::ARROW_UP;
pub const MOVE_DOWN: &str = phosphor::ARROW_DOWN;

pub fn command(command: Command) -> &'static str {
    match command {
        Command::Palette => SEARCH,
        Command::New => phosphor::FILE_PLUS,
        Command::Open => phosphor::FOLDER_OPEN,
        Command::Save => phosphor::FLOPPY_DISK,
        Command::SaveAs => phosphor::FLOPPY_DISK_BACK,
        Command::VersionHistory => phosphor::CLOCK_COUNTER_CLOCKWISE,
        Command::Import => phosphor::DOWNLOAD_SIMPLE,
        Command::Export => phosphor::EXPORT,
        Command::Preferences => phosphor::GEAR,
        Command::KeyboardShortcuts => phosphor::KEYBOARD,
        Command::Quit => phosphor::SIGN_OUT,
        Command::Undo => phosphor::ARROW_U_UP_LEFT,
        Command::Redo => phosphor::ARROW_U_UP_RIGHT,
        Command::NewSketch => phosphor::PENCIL_LINE,
        Command::FinishSketch => phosphor::CHECK,
        Command::SketchTool(tool) => self::tool(tool),
        Command::Constraint(tool) => constraint(tool),
        Command::DeleteSelection => DELETE,
        Command::Extrude => EXTRUDE,
        Command::Revolve => REVOLVE,
        Command::Fillet => blend(BlendKind::Fillet),
        Command::Chamfer => blend(BlendKind::Chamfer),
        Command::Shell => SHELL,
        Command::DatumPlane => PLANE,
        Command::DatumAxis => AXIS,
        Command::FitView => phosphor::FRAME_CORNERS,
        Command::LargerInterface => phosphor::MAGNIFYING_GLASS_PLUS,
        Command::SmallerInterface => phosphor::MAGNIFYING_GLASS_MINUS,
        Command::NormalInterface => phosphor::TEXT_AA,
        Command::View(_) => phosphor::CUBE_FOCUS,
        Command::Camera(camera) => camera_move(camera),
        Command::HighlightNext => phosphor::ARROW_RIGHT,
        Command::HighlightPrevious => phosphor::ARROW_LEFT,
        Command::ActivateHighlighted => phosphor::CURSOR_CLICK,
        Command::OpenSample(_) => SAMPLE,
        Command::Welcome => phosphor::HAND_WAVING,
    }
}

const EXTRUDE: &str = phosphor::ARROW_FAT_LINE_UP;
const REVOLVE: &str = phosphor::ARROWS_CLOCKWISE;
const SHELL: &str = phosphor::CUBE_TRANSPARENT;
const PLANE: &str = phosphor::PARALLELOGRAM;
const AXIS: &str = phosphor::ARROW_LINE_UP_RIGHT;
const SKETCH: &str = phosphor::PENCIL_LINE;
const IMPORTED: &str = phosphor::CUBE;

pub fn tool(tool: Tool) -> &'static str {
    match tool {
        Tool::Select => phosphor::CURSOR,
        Tool::Point => phosphor::DOT_OUTLINE,
        Tool::Line => phosphor::LINE_SEGMENT,
        Tool::Rectangle => phosphor::RECTANGLE,
        Tool::Circle => phosphor::CIRCLE,
        Tool::Arc => phosphor::CIRCLE_HALF,
        Tool::Spline => phosphor::BEZIER_CURVE,
    }
}

pub fn constraint(tool: ConstraintTool) -> &'static str {
    match tool {
        ConstraintTool::Coincident => phosphor::CROSSHAIR_SIMPLE,
        ConstraintTool::Horizontal => phosphor::ARROWS_OUT_LINE_HORIZONTAL,
        ConstraintTool::Vertical => phosphor::ARROWS_OUT_LINE_VERTICAL,
        ConstraintTool::Parallel => phosphor::PAUSE,
        ConstraintTool::Perpendicular => phosphor::ANGLE,
        ConstraintTool::Tangent => phosphor::ARROW_ARC_RIGHT,
        ConstraintTool::Equal => phosphor::EQUALS,
        ConstraintTool::Distance => phosphor::RULER,
        ConstraintTool::Angle => phosphor::COMPASS_TOOL,
        ConstraintTool::Radius => phosphor::CIRCLE_DASHED,
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

pub fn feature(kind: &FeatureKind) -> &'static str {
    match kind {
        FeatureKind::Sketch(_) => SKETCH,
        FeatureKind::Solid(SolidFeature::Extrude(_)) => EXTRUDE,
        FeatureKind::Solid(SolidFeature::Revolve(_)) => REVOLVE,
        FeatureKind::Blend(blend) => self::blend(blend.kind),
        FeatureKind::Shell(_) => SHELL,
        FeatureKind::Datum(Datum::Plane(_)) => PLANE,
        FeatureKind::Datum(Datum::Axis(_)) => AXIS,
        FeatureKind::Import(_) => IMPORTED,
    }
}
