use std::collections::BTreeMap;

use caditor_file::Settings;
use caditor_geometry::Vector3;
use egui::{Event, Key, KeyboardShortcut, Modifiers};

use crate::{
    analysis::AnalysisCommand, display_style::DisplayStyle, editing::Tool, samples::Sample,
    selection::SelectionFilter, shape_modes::ShapeMode, sketch_tools::ConstraintTool,
    variants::all_variants,
};

const SETTINGS_PREFIX: &str = "keys.";
const RESERVED_KEYS: [Key; 3] = [Key::Escape, Key::Enter, Key::Tab];
const MODIFIER_KEYS: [Key; 8] = [
    Key::ShiftLeft,
    Key::ShiftRight,
    Key::ControlLeft,
    Key::ControlRight,
    Key::AltLeft,
    Key::AltRight,
    Key::SuperLeft,
    Key::SuperRight,
];
const WIDGET_KEYS: [Key; 12] = [
    Key::Space,
    Key::Enter,
    Key::Tab,
    Key::Escape,
    Key::ArrowUp,
    Key::ArrowDown,
    Key::ArrowLeft,
    Key::ArrowRight,
    Key::PageUp,
    Key::PageDown,
    Key::Home,
    Key::End,
];
const KEPT_WHILE_DRAWING: [Key; 2] = [Key::Backspace, Key::Delete];
const TEXT_EDITING_KEYS: [Key; 14] = [
    Key::A,
    Key::C,
    Key::V,
    Key::X,
    Key::Y,
    Key::Z,
    Key::ArrowUp,
    Key::ArrowDown,
    Key::ArrowLeft,
    Key::ArrowRight,
    Key::Home,
    Key::End,
    Key::Backspace,
    Key::Delete,
];
const CTRL: &str = "Ctrl";
const ALT: &str = "Alt";
const SHIFT: &str = "Shift";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Command {
    Palette,
    New,
    NewFromTemplate,
    Open,
    Save,
    SaveAs,
    SaveAsTemplate,
    VersionHistory,
    ModelProperties,
    Import,
    Export,
    ExportImage,
    ImportParameters,
    ExportParameters,
    ExportSketch,
    ExportFace,
    KeepDrawingConstruction,
    Preferences,
    KeyboardShortcuts,
    Quit,
    Undo,
    UndoHistory,
    Redo,
    NewSketch,
    FinishSketch,
    ReverseArc,
    MoreSides,
    FewerSides,
    Construction,
    ToggleConstraintActive,
    SplitCurve,
    BreakCurves,
    RotateGeometry,
    ScaleGeometry,
    MoveGeometry,
    SelectAll,
    SelectFree,
    CopyGeometry,
    CutGeometry,
    PasteGeometry,
    IntersectBody,
    SketchTool(Tool),
    ShapeMode(ShapeMode),
    Constraint(ConstraintTool),
    DeleteSelection,
    Extrude,
    Revolve,
    Hole,
    NewBox,
    NewCylinder,
    NewSphere,
    NewTorus,
    Thread,
    NewCone,
    NewWedge,
    NewPrism,
    Fillet,
    Chamfer,
    Shell,
    OffsetFace,
    Combine,
    Move,
    CopyBody,
    Mirror,
    Split,
    Scale,
    BodyAppearance,
    RenameBody,
    RemoveBody,
    LinearPattern,
    CircularPattern,
    DatumPlane,
    DatumAxis,
    DatumPoint,
    CoordinateSystem,
    ScaleModel,
    FitView,
    Measure,
    Interference,
    LargerInterface,
    SmallerInterface,
    NormalInterface,
    View(StandardView),
    Camera(CameraMove),
    Filter(SelectionFilter),
    Style(DisplayStyle),
    Analysis(AnalysisCommand),
    HighlightNext,
    HighlightPrevious,
    ActivateHighlighted,
    ListUnderPointer,
    HideSelection,
    HideOthers,
    LookAtFace,
    LookAtSketch,
    SelectAllShapes,
    SelectTangentEdges,
    SelectTangentFaces,
    SelectHole,
    SelectBody,
    SelectFaceEdges,
    SaveSelectionSet,
    SelectionSets,
    ToggleVisibility,
    ShowAll,
    TogglePrincipal,
    ToggleSketches,
    ToggleDatums,
    ToggleBodies,
    SaveView,
    SavedViews,
    SetHomeView,
    ResetHomeView,
    ToggleProjection,
    AutomaticProjection,
    ToggleSnapping,
    ToggleGridSnapping,
    ToggleLasso,
    TogglePaintSelection,
    ToggleSelectThrough,
    CycleSelectionPriority,
    ToggleTypedDimensions,
    ToggleFirstDimensionScales,
    ToggleGlyphs,
    ToggleCentresOfMass,
    MinimizeWindow,
    MaximizeWindow,
    FullScreen,
    OpenSample(Sample),
    OpenRecent(RecentSlot),
    ClearRecent,
    RecoverUnsaved,
    CancelExport,
    CancelImageExport,
    CancelImport,
    Recompute,
    CancelRecompute,
    RenameFeature,
    GroupFeatures,
    Ungroup,
    RenameGroup,
    MoveFeatureUp,
    MoveFeatureDown,
    DeleteFeature,
    CopyFeatures,
    PasteFeatures,
    SuppressFeature,
    RollToHere,
    RollToEnd,
    RollbackUp,
    RollbackDown,
    EditFeature,
    CloseFeature,
    DetachSketch,
    PlaceSketch,
    UseSelectedAxis,
    ExtrudeUpToSelected,
    ClearChosenRegions,
    StartAtSelected,
    MirrorAcrossSelected,
    SplitAlongSelected,
    DatumUseSelected,
    DatumTurnAboutSelected,
    PatternUseSelected,
    PatternSecondUseSelected,
    MoveTurnAboutSelected,
    FilterFeatures,
    AddParameter,
    DeleteParameter,
    MoveParameterUp,
    MoveParameterDown,
    ParameterNote,
    ShowFirstFailed,
    UpdateReferences,
    ReplaceImport,
    ReloadImport,
    DismissNotice,
    DismissTip,
    HideTips,
    Welcome,
    About,
    Messages,
    Mate,
}

impl Command {
    pub fn repeats(self) -> bool {
        matches!(
            self,
            Self::Camera(_)
                | Self::HighlightNext
                | Self::HighlightPrevious
                | Self::Undo
                | Self::Redo
                | Self::LargerInterface
                | Self::SmallerInterface
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RecentSlot(usize);

const RECENT_IDS: [&str; 10] = [
    "file.recent_1",
    "file.recent_2",
    "file.recent_3",
    "file.recent_4",
    "file.recent_5",
    "file.recent_6",
    "file.recent_7",
    "file.recent_8",
    "file.recent_9",
    "file.recent_10",
];

impl RecentSlot {
    pub const ALL: [Self; 10] = [
        Self(0),
        Self(1),
        Self(2),
        Self(3),
        Self(4),
        Self(5),
        Self(6),
        Self(7),
        Self(8),
        Self(9),
    ];

    pub fn index(self) -> usize {
        self.0
    }

    fn id(self) -> &'static str {
        RECENT_IDS.get(self.0).copied().unwrap_or("file.recent")
    }

    fn title(self) -> String {
        match self.0 {
            0 => "Open the most recent model".to_owned(),
            index => format!("Open recent model {}", index + 1),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StandardView {
    Isometric,
    Front,
    Top,
    Right,
    Back,
    Bottom,
    Left,
}

all_variants!(StandardView: Isometric, Front, Top, Right, Back, Bottom, Left);

impl StandardView {
    pub fn looking_from(self) -> Vector3 {
        match self {
            Self::Isometric => Vector3::new(1.0, -1.0, 1.0),
            Self::Front => Vector3::NEG_Y,
            Self::Top => Vector3::Z,
            Self::Right => Vector3::X,
            Self::Back => Vector3::Y,
            Self::Bottom => Vector3::NEG_Z,
            Self::Left => Vector3::NEG_X,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Isometric => "isometric",
            Self::Front => "front",
            Self::Top => "top",
            Self::Right => "right",
            Self::Back => "back",
            Self::Bottom => "bottom",
            Self::Left => "left",
        }
    }

    fn key(self) -> Key {
        match self {
            Self::Isometric => Key::Num0,
            Self::Front => Key::Num1,
            Self::Top => Key::Num2,
            Self::Right => Key::Num3,
            Self::Back => Key::Num4,
            Self::Bottom => Key::Num5,
            Self::Left => Key::Num6,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CameraMove {
    OrbitLeft,
    OrbitRight,
    OrbitUp,
    OrbitDown,
    PanLeft,
    PanRight,
    PanUp,
    PanDown,
    ZoomIn,
    ZoomOut,
}

all_variants!(CameraMove: OrbitLeft, OrbitRight, OrbitUp, OrbitDown, PanLeft, PanRight, PanUp, PanDown, ZoomIn, ZoomOut);

impl CameraMove {
    fn id(self) -> &'static str {
        match self {
            Self::OrbitLeft => "view.orbit_left",
            Self::OrbitRight => "view.orbit_right",
            Self::OrbitUp => "view.orbit_up",
            Self::OrbitDown => "view.orbit_down",
            Self::PanLeft => "view.pan_left",
            Self::PanRight => "view.pan_right",
            Self::PanUp => "view.pan_up",
            Self::PanDown => "view.pan_down",
            Self::ZoomIn => "view.zoom_in",
            Self::ZoomOut => "view.zoom_out",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::OrbitLeft => "Orbit left",
            Self::OrbitRight => "Orbit right",
            Self::OrbitUp => "Orbit up",
            Self::OrbitDown => "Orbit down",
            Self::PanLeft => "Pan left",
            Self::PanRight => "Pan right",
            Self::PanUp => "Pan up",
            Self::PanDown => "Pan down",
            Self::ZoomIn => "Zoom in",
            Self::ZoomOut => "Zoom out",
        }
    }

    fn shortcut(self) -> KeyboardShortcut {
        let (modifiers, key) = match self {
            Self::OrbitLeft => (Modifiers::NONE, Key::ArrowLeft),
            Self::OrbitRight => (Modifiers::NONE, Key::ArrowRight),
            Self::OrbitUp => (Modifiers::NONE, Key::ArrowUp),
            Self::OrbitDown => (Modifiers::NONE, Key::ArrowDown),
            Self::PanLeft => (Modifiers::SHIFT, Key::ArrowLeft),
            Self::PanRight => (Modifiers::SHIFT, Key::ArrowRight),
            Self::PanUp => (Modifiers::SHIFT, Key::ArrowUp),
            Self::PanDown => (Modifiers::SHIFT, Key::ArrowDown),
            Self::ZoomIn => (Modifiers::NONE, Key::PageUp),
            Self::ZoomOut => (Modifiers::NONE, Key::PageDown),
        };
        KeyboardShortcut::new(modifiers, key)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Category {
    File,
    Edit,
    View,
    Model,
    Sketch,
    Constraint,
    Help,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Self::File => "File",
            Self::Edit => "Edit",
            Self::View => "View",
            Self::Model => "Model",
            Self::Sketch => "Sketch",
            Self::Constraint => "Constraint",
            Self::Help => "Help",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Anywhere,
    Sketch,
    OutsideSketch,
}

impl Scope {
    fn overlaps(self, other: Self) -> bool {
        self == Self::Anywhere || other == Self::Anywhere || self == other
    }

    pub fn describe(self) -> &'static str {
        match self {
            Self::Anywhere => "anywhere",
            Self::Sketch => "while editing a sketch",
            Self::OutsideSketch => "outside sketch editing",
        }
    }
}

macro_rules! plain_commands {
    ($($variant:ident,)*) => {
        const PLAIN_COMMANDS: &[Command] = &[$(Command::$variant,)*];

        const _: fn(Command) = |command| match command {
            $(Command::$variant)|* => {}
            Command::SketchTool(_)
            | Command::ShapeMode(_)
            | Command::Constraint(_)
            | Command::View(_)
            | Command::Camera(_)
            | Command::Filter(_)
            | Command::Style(_)
            | Command::Analysis(_)
            | Command::OpenSample(_)
            | Command::OpenRecent(_) => {}
        };
    };
}

plain_commands! {
    Palette,
    New,
    NewFromTemplate,
    Open,
    Save,
    SaveAs,
    SaveAsTemplate,
    VersionHistory,
    ModelProperties,
    Import,
    Export,
    ExportImage,
    ImportParameters,
    ExportParameters,
    ExportSketch,
    ExportFace,
    KeepDrawingConstruction,
    Preferences,
    KeyboardShortcuts,
    Welcome,
    About,
    Messages,
    Quit,
    Undo,
    UndoHistory,
    Redo,
    FitView,
    ToggleProjection,
    AutomaticProjection,
    ToggleSnapping,
    ToggleGridSnapping,
    ToggleLasso,
    TogglePaintSelection,
    ToggleSelectThrough,
    CycleSelectionPriority,
    ToggleTypedDimensions,
    ToggleFirstDimensionScales,
    ToggleGlyphs,
    ToggleCentresOfMass,
    Measure,
    Interference,
    LargerInterface,
    SmallerInterface,
    NormalInterface,
    HighlightNext,
    HighlightPrevious,
    ActivateHighlighted,
    ListUnderPointer,
    HideSelection,
    HideOthers,
    LookAtFace,
    LookAtSketch,
    SelectAllShapes,
    SelectTangentEdges,
    SelectTangentFaces,
    SelectHole,
    SelectBody,
    SelectFaceEdges,
    SaveSelectionSet,
    SelectionSets,
    ToggleVisibility,
    ShowAll,
    TogglePrincipal,
    ToggleSketches,
    ToggleDatums,
    ToggleBodies,
    SaveView,
    SavedViews,
    SetHomeView,
    ResetHomeView,
    MinimizeWindow,
    MaximizeWindow,
    FullScreen,
    NewSketch,
    Extrude,
    Revolve,
    Hole,
    NewBox,
    NewCylinder,
    NewSphere,
    NewTorus,
    Thread,
    NewCone,
    NewWedge,
    NewPrism,
    Fillet,
    Chamfer,
    Shell,
    OffsetFace,
    Combine,
    Move,
    CopyBody,
    Mirror,
    Split,
    Scale,
    BodyAppearance,
    RenameBody,
    RemoveBody,
    LinearPattern,
    CircularPattern,
    DatumPlane,
    DatumAxis,
    DatumPoint,
    CoordinateSystem,
    ScaleModel,
    FinishSketch,
    ReverseArc,
    MoreSides,
    FewerSides,
    Construction,
    ToggleConstraintActive,
    SplitCurve,
    BreakCurves,
    RotateGeometry,
    ScaleGeometry,
    MoveGeometry,
    SelectAll,
    SelectFree,
    CopyGeometry,
    CutGeometry,
    PasteGeometry,
    IntersectBody,
    DeleteSelection,
    ClearRecent,
    RecoverUnsaved,
    CancelExport,
    CancelImageExport,
    CancelImport,
    Recompute,
    CancelRecompute,
    RenameFeature,
    GroupFeatures,
    Ungroup,
    RenameGroup,
    MoveFeatureUp,
    MoveFeatureDown,
    DeleteFeature,
    CopyFeatures,
    PasteFeatures,
    SuppressFeature,
    RollToHere,
    RollToEnd,
    RollbackUp,
    RollbackDown,
    EditFeature,
    CloseFeature,
    DetachSketch,
    PlaceSketch,
    UseSelectedAxis,
    ExtrudeUpToSelected,
    ClearChosenRegions,
    StartAtSelected,
    MirrorAcrossSelected,
    SplitAlongSelected,
    DatumUseSelected,
    DatumTurnAboutSelected,
    PatternUseSelected,
    PatternSecondUseSelected,
    MoveTurnAboutSelected,
    FilterFeatures,
    AddParameter,
    DeleteParameter,
    MoveParameterUp,
    MoveParameterDown,
    ParameterNote,
    ShowFirstFailed,
    UpdateReferences,
    ReplaceImport,
    ReloadImport,
    DismissNotice,
    DismissTip,
    HideTips,
    Mate,
}

impl Command {
    pub fn all() -> impl Iterator<Item = Self> {
        PLAIN_COMMANDS
            .iter()
            .copied()
            .chain(Tool::ALL.into_iter().map(Self::SketchTool))
            .chain(ShapeMode::ALL.into_iter().map(Self::ShapeMode))
            .chain(ConstraintTool::ALL.into_iter().map(Self::Constraint))
            .chain(StandardView::ALL.into_iter().map(Self::View))
            .chain(CameraMove::ALL.into_iter().map(Self::Camera))
            .chain(SelectionFilter::ALL.into_iter().map(Self::Filter))
            .chain(DisplayStyle::ALL.into_iter().map(Self::Style))
            .chain(AnalysisCommand::ALL.into_iter().map(Self::Analysis))
            .chain(Sample::ALL.into_iter().map(Self::OpenSample))
            .chain(RecentSlot::ALL.into_iter().map(Self::OpenRecent))
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Palette => "palette",
            Self::New => "file.new",
            Self::NewFromTemplate => "file.new_from_template",
            Self::SaveAsTemplate => "file.save_as_template",
            Self::Open => "file.open",
            Self::Save => "file.save",
            Self::SaveAs => "file.save_as",
            Self::VersionHistory => "file.history",
            Self::ModelProperties => "file.properties",
            Self::Import => "file.import",
            Self::Export => "file.export",
            Self::ExportImage => "file.export_image",
            Self::ImportParameters => "file.import_parameters",
            Self::ExportParameters => "file.export_parameters",
            Self::ExportSketch => "file.export_sketch",
            Self::ExportFace => "file.export_face",
            Self::KeepDrawingConstruction => "file.keep_drawing_construction",
            Self::Preferences => "file.preferences",
            Self::KeyboardShortcuts => "file.shortcuts",
            Self::Quit => "file.quit",
            Self::Undo => "edit.undo",
            Self::UndoHistory => "edit.undo_history",
            Self::Redo => "edit.redo",
            Self::NewSketch => "model.new_sketch",
            Self::FinishSketch => "sketch.finish",
            Self::ReverseArc => "sketch.reverse_arc",
            Self::MoreSides => "sketch.more_sides",
            Self::FewerSides => "sketch.fewer_sides",
            Self::Construction => "sketch.construction",
            Self::ToggleConstraintActive => "sketch.toggle_constraint_active",
            Self::SplitCurve => "sketch.split_curve",
            Self::BreakCurves => "sketch.break_curves",
            Self::RotateGeometry => "sketch.rotate",
            Self::ScaleGeometry => "sketch.scale",
            Self::MoveGeometry => "sketch.move",
            Self::SelectAll => "sketch.select_all",
            Self::SelectFree => "sketch.select_free",
            Self::CopyGeometry => "sketch.copy",
            Self::CutGeometry => "sketch.cut",
            Self::PasteGeometry => "sketch.paste",
            Self::IntersectBody => "sketch.intersect_body",
            Self::SketchTool(tool) => match tool {
                Tool::Select => "sketch.select",
                Tool::Point => "sketch.point",
                Tool::Line => "sketch.line",
                Tool::Rectangle => "sketch.rectangle",
                Tool::Circle => "sketch.circle",
                Tool::Arc => "sketch.arc",
                Tool::ThreePointArc => "sketch.three_point_arc",
                Tool::TangentArc => "sketch.tangent_arc",
                Tool::Ellipse => "sketch.ellipse",
                Tool::EllipticalArc => "sketch.elliptical_arc",
                Tool::Slot => "sketch.slot",
                Tool::Polygon => "sketch.polygon",
                Tool::Spline => "sketch.spline",
                Tool::Trim => "sketch.trim",
                Tool::Extend => "sketch.extend",
                Tool::Offset => "sketch.offset",
                Tool::Mirror => "sketch.mirror",
                Tool::RectangularPattern => "sketch.rectangular_pattern",
                Tool::CircularPattern => "sketch.circular_pattern",
                Tool::TangentCircle => "sketch.tangent_circle",
                Tool::Fillet => "sketch.fillet",
                Tool::Chamfer => "sketch.chamfer",
                Tool::Project => "sketch.project",
                Tool::Intersect => "sketch.intersect",
                Tool::Dimension => "sketch.dimension",
                Tool::BlendCurve => "sketch.blend_curve",
            },
            Self::ShapeMode(mode) => mode.id(),
            Self::Filter(filter) => filter.id(),
            Self::Style(style) => style.id(),
            Self::Analysis(analysis) => analysis.id(),
            Self::Constraint(tool) => match tool {
                ConstraintTool::Coincident => "constraint.coincident",
                ConstraintTool::Midpoint => "constraint.midpoint",
                ConstraintTool::Concentric => "constraint.concentric",
                ConstraintTool::Collinear => "constraint.collinear",
                ConstraintTool::Fix => "constraint.fix",
                ConstraintTool::Horizontal => "constraint.horizontal",
                ConstraintTool::Vertical => "constraint.vertical",
                ConstraintTool::Parallel => "constraint.parallel",
                ConstraintTool::Perpendicular => "constraint.perpendicular",
                ConstraintTool::Tangent => "constraint.tangent",
                ConstraintTool::Curvature => "constraint.curvature",
                ConstraintTool::Equal => "constraint.equal",
                ConstraintTool::Symmetric => "constraint.symmetric",
                ConstraintTool::Distance => "constraint.distance",
                ConstraintTool::HorizontalDistance => "constraint.horizontal_distance",
                ConstraintTool::VerticalDistance => "constraint.vertical_distance",
                ConstraintTool::Angle => "constraint.angle",
                ConstraintTool::Radius => "constraint.radius",
                ConstraintTool::Diameter => "constraint.diameter",
            },
            Self::DeleteSelection => "edit.delete",
            Self::Extrude => "model.extrude",
            Self::Revolve => "model.revolve",
            Self::Hole => "model.hole",
            Self::NewBox => "model.box",
            Self::NewCylinder => "model.cylinder",
            Self::NewSphere => "model.sphere",
            Self::NewTorus => "model.torus",
            Self::Thread => "model.thread",
            Self::NewCone => "model.cone",
            Self::NewWedge => "model.wedge",
            Self::NewPrism => "model.prism",
            Self::Fillet => "model.fillet",
            Self::Chamfer => "model.chamfer",
            Self::Shell => "model.shell",
            Self::OffsetFace => "model.offset_face",
            Self::Combine => "model.combine",
            Self::Move => "model.move",
            Self::CopyBody => "model.copy_body",
            Self::Mirror => "model.mirror",
            Self::Split => "model.split",
            Self::Scale => "model.scale",
            Self::BodyAppearance => "model.body_appearance",
            Self::RenameBody => "model.rename_body",
            Self::RemoveBody => "model.remove_body",
            Self::LinearPattern => "model.linear_pattern",
            Self::CircularPattern => "model.circular_pattern",
            Self::DatumPlane => "model.plane",
            Self::DatumAxis => "model.axis",
            Self::DatumPoint => "model.point",
            Self::CoordinateSystem => "model.coordinate_system",
            Self::ScaleModel => "model.scale_model",
            Self::FitView => "view.fit",
            Self::Measure => "view.measure",
            Self::Interference => "view.interference",
            Self::LargerInterface => "view.interface_larger",
            Self::SmallerInterface => "view.interface_smaller",
            Self::NormalInterface => "view.interface_normal",
            Self::View(view) => match view {
                StandardView::Isometric => "view.isometric",
                StandardView::Front => "view.front",
                StandardView::Top => "view.top",
                StandardView::Right => "view.right",
                StandardView::Back => "view.back",
                StandardView::Bottom => "view.bottom",
                StandardView::Left => "view.left",
            },
            Self::Camera(camera) => camera.id(),
            Self::HighlightNext => "view.highlight_next",
            Self::HighlightPrevious => "view.highlight_previous",
            Self::ActivateHighlighted => "view.activate_highlighted",
            Self::ListUnderPointer => "view.list_under_pointer",
            Self::HideSelection => "view.hide_selection",
            Self::HideOthers => "view.hide_others",
            Self::LookAtFace => "view.look_at_face",
            Self::LookAtSketch => "view.look_at_sketch",
            Self::SelectAllShapes => "select.all",
            Self::SelectTangentEdges => "select.tangent_edges",
            Self::SelectTangentFaces => "select.tangent_faces",
            Self::SelectHole => "select.hole",
            Self::SelectBody => "select.body",
            Self::SaveSelectionSet => "select.save_set",
            Self::SelectionSets => "select.sets",
            Self::SelectFaceEdges => "select.face_edges",
            Self::ToggleVisibility => "view.toggle_visibility",
            Self::ShowAll => "view.show_all",
            Self::TogglePrincipal => "view.toggle_principal",
            Self::ToggleSketches => "view.toggle_sketches",
            Self::ToggleDatums => "view.toggle_datums",
            Self::ToggleBodies => "view.toggle_bodies",
            Self::SaveView => "view.save",
            Self::SavedViews => "view.saved_views",
            Self::SetHomeView => "view.set_home",
            Self::ResetHomeView => "view.reset_home",
            Self::ToggleProjection => "view.toggle_projection",
            Self::AutomaticProjection => "view.automatic_projection",
            Self::ToggleSnapping => "view.toggle_snapping",
            Self::ToggleGridSnapping => "view.toggle_grid_snapping",
            Self::ToggleLasso => "view.toggle_lasso",
            Self::TogglePaintSelection => "view.toggle_paint_selection",
            Self::ToggleSelectThrough => "view.toggle_select_through",
            Self::CycleSelectionPriority => "select.priority",
            Self::ToggleTypedDimensions => "sketch.toggle_typed_dimensions",
            Self::ToggleFirstDimensionScales => "sketch.toggle_first_dimension_scales",
            Self::ToggleGlyphs => "view.toggle_glyphs",
            Self::ToggleCentresOfMass => "view.toggle_centres_of_mass",
            Self::MinimizeWindow => "view.minimize_window",
            Self::MaximizeWindow => "view.maximize_window",
            Self::FullScreen => "view.full_screen",
            Self::OpenSample(sample) => match sample {
                Sample::Plate => "file.sample.plate",
                Sample::Spool => "file.sample.spool",
                Sample::Bracket => "file.sample.bracket",
            },
            Self::OpenRecent(slot) => slot.id(),
            Self::ClearRecent => "file.clear_recent",
            Self::RecoverUnsaved => "file.recover",
            Self::CancelExport => "file.cancel_export",
            Self::CancelImageExport => "file.cancel_image_export",
            Self::CancelImport => "file.cancel_import",
            Self::Recompute => "model.recompute",
            Self::CancelRecompute => "model.cancel_recompute",
            Self::RenameFeature => "model.rename_feature",
            Self::GroupFeatures => "model.group_features",
            Self::Ungroup => "model.ungroup",
            Self::RenameGroup => "model.rename_group",
            Self::MoveFeatureUp => "model.move_feature_up",
            Self::MoveFeatureDown => "model.move_feature_down",
            Self::DeleteFeature => "model.delete_feature",
            Self::CopyFeatures => "model.copy_features",
            Self::PasteFeatures => "model.paste_features",
            Self::SuppressFeature => "model.suppress_feature",
            Self::RollToHere => "model.roll_to_here",
            Self::RollToEnd => "model.roll_to_end",
            Self::RollbackUp => "model.rollback_up",
            Self::RollbackDown => "model.rollback_down",
            Self::EditFeature => "model.edit_feature",
            Self::CloseFeature => "model.close_feature",
            Self::DetachSketch => "model.detach_sketch",
            Self::PlaceSketch => "model.place_sketch",
            Self::UseSelectedAxis => "model.use_selected_axis",
            Self::ExtrudeUpToSelected => "model.extrude_up_to_selected",
            Self::ClearChosenRegions => "model.clear_chosen_regions",
            Self::StartAtSelected => "model.start_at_selected",
            Self::MirrorAcrossSelected => "model.mirror_across_selected",
            Self::SplitAlongSelected => "model.split_along_selected",
            Self::DatumUseSelected => "model.datum_use_selected",
            Self::DatumTurnAboutSelected => "model.datum_turn_about_selected",
            Self::PatternUseSelected => "model.pattern_use_selected",
            Self::PatternSecondUseSelected => "model.pattern_second_direction",
            Self::MoveTurnAboutSelected => "model.move_turn_about_selected",
            Self::FilterFeatures => "model.filter_features",
            Self::AddParameter => "model.add_parameter",
            Self::DeleteParameter => "model.delete_parameter",
            Self::MoveParameterUp => "model.move_parameter_up",
            Self::MoveParameterDown => "model.move_parameter_down",
            Self::ParameterNote => "model.parameter_note",
            Self::ShowFirstFailed => "model.first_failed",
            Self::UpdateReferences => "model.update_references",
            Self::ReplaceImport => "file.replace_import",
            Self::ReloadImport => "file.reload_import",
            Self::DismissNotice => "edit.dismiss_notice",
            Self::DismissTip => "help.dismiss_tip",
            Self::HideTips => "help.hide_tips",
            Self::Welcome => "help.welcome",
            Self::About => "help.about",
            Self::Messages => "help.messages",
            Self::Mate => "model.mate",
        }
    }

    pub fn title(self) -> String {
        let fixed = match self {
            Self::Palette => "Search commands",
            Self::New => "New model",
            Self::NewFromTemplate => "New from template…",
            Self::SaveAsTemplate => "Save as template…",
            Self::Open => "Open…",
            Self::Save => "Save",
            Self::SaveAs => "Save as…",
            Self::VersionHistory => "Version history…",
            Self::ModelProperties => "Model properties…",
            Self::Import => "Import…",
            Self::Export => "Export…",
            Self::ExportImage => "Export image…",
            Self::ImportParameters => "Import parameters…",
            Self::ExportParameters => "Export parameters…",
            Self::ExportSketch => "Export sketch…",
            Self::ExportFace => "Export face…",
            Self::KeepDrawingConstruction => "Keep construction geometry in drawings",
            Self::Preferences => "Preferences…",
            Self::KeyboardShortcuts => "Keyboard shortcuts…",
            Self::Quit => "Quit",
            Self::Undo => "Undo",
            Self::UndoHistory => "Undo history…",
            Self::Redo => "Redo",
            Self::NewSketch => "New sketch",
            Self::FinishSketch => "Finish sketch",
            Self::ReverseArc => "Reverse the arc",
            Self::MoreSides => "Give the polygon another side",
            Self::FewerSides => "Give the polygon one side fewer",
            Self::Construction => "Switch to or from construction geometry",
            Self::ToggleConstraintActive => "Disable or enable the selected constraints",
            Self::SplitCurve => "Split the selected curve at the selected point",
            Self::BreakCurves => "Break the selected curves at every crossing",
            Self::RotateGeometry => "Rotate selected sketch geometry",
            Self::ScaleGeometry => "Scale selected sketch geometry",
            Self::MoveGeometry => "Move selected sketch geometry",
            Self::SelectAll => "Select all sketch geometry",
            Self::SelectFree => "Select what is still free in the sketch",
            Self::CopyGeometry => "Copy selected sketch geometry",
            Self::CutGeometry => "Cut selected sketch geometry",
            Self::PasteGeometry => "Paste sketch geometry",
            Self::IntersectBody => "Intersect the whole body of the highlighted face",
            Self::SketchTool(Tool::Select) => "Select tool",
            Self::SketchTool(Tool::Trim) => "Trim sketch curves",
            Self::SketchTool(Tool::Extend) => "Extend a line or arc",
            Self::SketchTool(Tool::Offset) => "Offset sketch curves",
            Self::SketchTool(Tool::Mirror) => "Mirror sketch geometry",
            Self::SketchTool(Tool::RectangularPattern) => "Repeat sketch geometry in a grid",
            Self::SketchTool(Tool::CircularPattern) => "Repeat sketch geometry about a point",
            Self::SketchTool(Tool::TangentCircle) => "Draw a circle tangent to sketch curves",
            Self::SketchTool(Tool::Fillet) => "Fillet a sketch corner",
            Self::SketchTool(Tool::Chamfer) => "Chamfer a sketch corner",
            Self::SketchTool(Tool::Project) => "Project model geometry into the sketch",
            Self::SketchTool(Tool::Intersect) => {
                "Draw where a body, a face or a datum plane cuts the sketch plane"
            }
            Self::SketchTool(Tool::Dimension) => "Smart dimension",
            Self::SketchTool(tool) => return format!("Draw {}", tool.label().to_lowercase()),
            Self::ShapeMode(mode) => return mode.title(),
            Self::Constraint(tool) => return tool.label().to_owned(),
            Self::DeleteSelection => "Delete selection",
            Self::Extrude => "Extrude",
            Self::Revolve => "Revolve",
            Self::Hole => "Hole",
            Self::NewBox => "Box",
            Self::NewCylinder => "Cylinder",
            Self::NewSphere => "Sphere",
            Self::NewTorus => "Torus",
            Self::Thread => "Thread",
            Self::NewCone => "Cone",
            Self::NewWedge => "Wedge",
            Self::NewPrism => "Prism",
            Self::Fillet => "Fillet",
            Self::Chamfer => "Chamfer",
            Self::Shell => "Shell",
            Self::OffsetFace => "Offset face",
            Self::Combine => "Combine",
            Self::Move => "Move body",
            Self::CopyBody => "Copy body",
            Self::Mirror => "Mirror body",
            Self::Split => "Split body",
            Self::Scale => "Scale body",
            Self::BodyAppearance => "Body colour and material",
            Self::RenameBody => "Rename body",
            Self::RemoveBody => "Remove body",
            Self::LinearPattern => "Linear pattern",
            Self::CircularPattern => "Circular pattern",
            Self::DatumPlane => "Datum plane",
            Self::DatumAxis => "Datum axis",
            Self::DatumPoint => "Datum point",
            Self::CoordinateSystem => "Coordinate system",
            Self::ScaleModel => "Scale model…",
            Self::FitView => "Fit view",
            Self::Measure => "Measure",
            Self::Interference => "Check interference",
            Self::LargerInterface => "Make the interface larger",
            Self::SmallerInterface => "Make the interface smaller",
            Self::NormalInterface => "Interface at normal size",
            Self::View(StandardView::Isometric) => "Isometric view",
            Self::View(view) => return format!("View from the {}", view.name()),
            Self::Camera(camera) => camera.title(),
            Self::Filter(filter) => filter.title(),
            Self::Style(style) => style.title(),
            Self::Analysis(analysis) => analysis.title(),
            Self::HighlightNext => "Highlight the next item in the view",
            Self::HighlightPrevious => "Highlight the previous item in the view",
            Self::ActivateHighlighted => "Select the highlighted item",
            Self::ListUnderPointer => "List everything under the pointer",
            Self::HideSelection => "Hide selection",
            Self::HideOthers => "Hide everything but the selection",
            Self::LookAtFace => "Look straight at the selected face",
            Self::LookAtSketch => "Look straight at the edited sketch",
            Self::SelectAllShapes => "Select all faces, edges or vertices",
            Self::SelectTangentEdges => "Select the edges tangent to the selected edges",
            Self::SelectTangentFaces => "Select the faces tangent to the selected faces",
            Self::SelectHole => "Select the whole hole of the selected wall",
            Self::SelectBody => "Select the whole body",
            Self::SaveSelectionSet => "Save the selection as a set",
            Self::SelectionSets => "Selection sets…",
            Self::SelectFaceEdges => "Select the edges around the selected faces",
            Self::ToggleVisibility => "Hide or show feature",
            Self::ShowAll => "Show everything",
            Self::TogglePrincipal => "Hide or show principal planes, axes and origin",
            Self::ToggleSketches => "Hide or show every sketch",
            Self::ToggleDatums => "Hide or show every datum",
            Self::ToggleBodies => "Hide or show every body",
            Self::SaveView => "Save the current view",
            Self::SavedViews => "Saved views…",
            Self::SetHomeView => "Make the current view the Isometric view",
            Self::ResetHomeView => "Reset the Isometric view",
            Self::ToggleProjection => "Switch between perspective and orthographic",
            Self::AutomaticProjection => "Perspective that turns orthographic in a standard view",
            Self::ToggleSnapping => "Turn snapping on or off",
            Self::ToggleGridSnapping => "Snap to the grid",
            Self::ToggleLasso => "Select with a lasso",
            Self::TogglePaintSelection => "Select faces by painting over them",
            Self::ToggleSelectThrough => "Select through to what is hidden",
            Self::CycleSelectionPriority => "Cycle the selection priority: body, face, edge",
            Self::ToggleTypedDimensions => "Keep typed values as dimensions",
            Self::ToggleFirstDimensionScales => "Scale the whole sketch on its first dimension",
            Self::ToggleGlyphs => "Show or hide constraint glyphs",
            Self::ToggleCentresOfMass => "Show or hide centres of mass",
            Self::MinimizeWindow => "Minimize the window",
            Self::MaximizeWindow => "Maximize or restore the window",
            Self::FullScreen => "Enter or leave full screen",
            Self::OpenSample(sample) => return format!("Open the {} sample", sample.title()),
            Self::OpenRecent(slot) => return slot.title(),
            Self::ClearRecent => "Clear recent files",
            Self::RecoverUnsaved => "Recover unsaved work…",
            Self::CancelExport => "Cancel the export",
            Self::CancelImageExport => "Cancel the image export",
            Self::CancelImport => "Cancel the import",
            Self::Recompute => "Recompute the model",
            Self::CancelRecompute => "Cancel the recompute",
            Self::RenameFeature => "Rename feature",
            Self::GroupFeatures => "Group features",
            Self::Ungroup => "Ungroup",
            Self::RenameGroup => "Rename group",
            Self::MoveFeatureUp => "Move feature up",
            Self::MoveFeatureDown => "Move feature down",
            Self::DeleteFeature => "Delete feature",
            Self::CopyFeatures => "Copy features",
            Self::PasteFeatures => "Paste features",
            Self::SuppressFeature => "Suppress or unsuppress feature",
            Self::RollToHere => "Roll back to here",
            Self::RollToEnd => "Roll to end",
            Self::RollbackUp => "Move the rollback bar up",
            Self::RollbackDown => "Move the rollback bar down",
            Self::EditFeature => "Edit feature",
            Self::CloseFeature => "Finish editing feature",
            Self::DetachSketch => "Detach sketch",
            Self::PlaceSketch => "Place sketch on selected plane or face",
            Self::UseSelectedAxis => "Revolve about selected axis",
            Self::ExtrudeUpToSelected => "Extrude up to selected face or plane",
            Self::ClearChosenRegions => "Clear the chosen regions",
            Self::StartAtSelected => "Start extrusion or revolution at selected face or plane",
            Self::MirrorAcrossSelected => "Mirror across selected face or plane",
            Self::SplitAlongSelected => "Split along selected plane, face, curve or body",
            Self::DatumUseSelected => "Base datum on selection",
            Self::DatumTurnAboutSelected => "Turn datum plane about selected axis",
            Self::PatternUseSelected => "Pattern along or about selected axis",
            Self::PatternSecondUseSelected => "Pattern also along selected direction",
            Self::MoveTurnAboutSelected => "Turn moved body about selected axis",
            Self::FilterFeatures => "Filter the feature tree",
            Self::AddParameter => "Add parameter",
            Self::DeleteParameter => "Delete parameter",
            Self::MoveParameterUp => "Move parameter up",
            Self::MoveParameterDown => "Move parameter down",
            Self::ParameterNote => "Add or edit the parameter's note",
            Self::ShowFirstFailed => "Go to the first failed feature",
            Self::UpdateReferences => "Update references",
            Self::ReplaceImport => "Replace import from file…",
            Self::ReloadImport => "Reload import from its file",
            Self::DismissNotice => "Dismiss the notice",
            Self::DismissTip => "Dismiss the tip",
            Self::HideTips => "Hide tips",
            Self::Welcome => "Welcome and samples…",
            Self::About => "About caditor",
            Self::Messages => "Recent messages",
            Self::Mate => "Mate body",
        };
        fixed.to_owned()
    }

    pub fn category(self) -> Category {
        match self {
            Self::New
            | Self::NewFromTemplate
            | Self::SaveAsTemplate
            | Self::Open
            | Self::Save
            | Self::SaveAs
            | Self::VersionHistory
            | Self::ModelProperties
            | Self::Import
            | Self::Export
            | Self::ExportImage
            | Self::ImportParameters
            | Self::ExportParameters
            | Self::ExportSketch
            | Self::ExportFace
            | Self::KeepDrawingConstruction
            | Self::Preferences
            | Self::KeyboardShortcuts
            | Self::Quit
            | Self::OpenSample(_)
            | Self::OpenRecent(_)
            | Self::ClearRecent
            | Self::RecoverUnsaved
            | Self::CancelExport
            | Self::CancelImageExport
            | Self::CancelImport => Category::File,
            Self::Welcome | Self::About | Self::Messages | Self::DismissTip | Self::HideTips => {
                Category::Help
            }
            Self::Palette
            | Self::Undo
            | Self::UndoHistory
            | Self::Redo
            | Self::DeleteSelection
            | Self::SelectAllShapes
            | Self::SelectTangentEdges
            | Self::SelectTangentFaces
            | Self::SelectHole
            | Self::SelectBody
            | Self::SelectFaceEdges
            | Self::SaveSelectionSet
            | Self::SelectionSets
            | Self::DismissNotice => Category::Edit,
            Self::FitView
            | Self::Measure
            | Self::Interference
            | Self::LargerInterface
            | Self::SmallerInterface
            | Self::NormalInterface
            | Self::View(_)
            | Self::Camera(_)
            | Self::Filter(_)
            | Self::Style(_)
            | Self::Analysis(_)
            | Self::HighlightNext
            | Self::HighlightPrevious
            | Self::ActivateHighlighted
            | Self::ListUnderPointer
            | Self::HideSelection
            | Self::HideOthers
            | Self::LookAtFace
            | Self::LookAtSketch
            | Self::ToggleVisibility
            | Self::ShowAll
            | Self::TogglePrincipal
            | Self::ToggleSketches
            | Self::ToggleDatums
            | Self::ToggleBodies
            | Self::SaveView
            | Self::SavedViews
            | Self::SetHomeView
            | Self::ResetHomeView
            | Self::ToggleProjection
            | Self::AutomaticProjection
            | Self::ToggleSnapping
            | Self::ToggleGridSnapping
            | Self::ToggleLasso
            | Self::TogglePaintSelection
            | Self::ToggleSelectThrough
            | Self::CycleSelectionPriority
            | Self::ToggleGlyphs
            | Self::ToggleCentresOfMass
            | Self::MinimizeWindow
            | Self::MaximizeWindow
            | Self::FullScreen => Category::View,
            Self::NewSketch
            | Self::Extrude
            | Self::Revolve
            | Self::Hole
            | Self::NewBox
            | Self::NewCylinder
            | Self::NewSphere
            | Self::NewTorus
            | Self::Thread
            | Self::NewCone
            | Self::NewWedge
            | Self::NewPrism
            | Self::Fillet
            | Self::Chamfer
            | Self::Shell
            | Self::OffsetFace
            | Self::Combine
            | Self::Move
            | Self::CopyBody
            | Self::Mirror
            | Self::Split
            | Self::Mate
            | Self::Scale
            | Self::BodyAppearance
            | Self::RenameBody
            | Self::RemoveBody
            | Self::LinearPattern
            | Self::CircularPattern
            | Self::DatumPlane
            | Self::DatumAxis
            | Self::DatumPoint
            | Self::CoordinateSystem
            | Self::ScaleModel
            | Self::Recompute
            | Self::CancelRecompute
            | Self::RenameFeature
            | Self::GroupFeatures
            | Self::Ungroup
            | Self::RenameGroup
            | Self::MoveFeatureUp
            | Self::MoveFeatureDown
            | Self::DeleteFeature
            | Self::CopyFeatures
            | Self::PasteFeatures
            | Self::SuppressFeature
            | Self::RollToHere
            | Self::RollToEnd
            | Self::RollbackUp
            | Self::RollbackDown
            | Self::EditFeature
            | Self::CloseFeature
            | Self::DetachSketch
            | Self::PlaceSketch
            | Self::UseSelectedAxis
            | Self::ExtrudeUpToSelected
            | Self::ClearChosenRegions
            | Self::StartAtSelected
            | Self::MirrorAcrossSelected
            | Self::SplitAlongSelected
            | Self::DatumUseSelected
            | Self::DatumTurnAboutSelected
            | Self::PatternUseSelected
            | Self::PatternSecondUseSelected
            | Self::MoveTurnAboutSelected
            | Self::FilterFeatures
            | Self::AddParameter
            | Self::DeleteParameter
            | Self::MoveParameterUp
            | Self::MoveParameterDown
            | Self::ParameterNote
            | Self::ShowFirstFailed
            | Self::UpdateReferences
            | Self::ReplaceImport
            | Self::ReloadImport => Category::Model,
            Self::FinishSketch
            | Self::ReverseArc
            | Self::MoreSides
            | Self::FewerSides
            | Self::Construction
            | Self::ToggleConstraintActive
            | Self::SplitCurve
            | Self::BreakCurves
            | Self::RotateGeometry
            | Self::ScaleGeometry
            | Self::ToggleTypedDimensions
            | Self::ToggleFirstDimensionScales
            | Self::MoveGeometry
            | Self::SelectAll
            | Self::SelectFree
            | Self::CopyGeometry
            | Self::CutGeometry
            | Self::PasteGeometry
            | Self::IntersectBody
            | Self::SketchTool(_)
            | Self::ShapeMode(_) => Category::Sketch,
            Self::Constraint(_) => Category::Constraint,
        }
    }

    pub fn scope(self) -> Scope {
        match self {
            Self::FinishSketch
            | Self::ReverseArc
            | Self::MoreSides
            | Self::FewerSides
            | Self::Construction
            | Self::ToggleConstraintActive
            | Self::SplitCurve
            | Self::BreakCurves
            | Self::RotateGeometry
            | Self::ScaleGeometry
            | Self::ToggleTypedDimensions
            | Self::ToggleFirstDimensionScales
            | Self::MoveGeometry
            | Self::SelectAll
            | Self::SelectFree
            | Self::CopyGeometry
            | Self::CutGeometry
            | Self::PasteGeometry
            | Self::IntersectBody
            | Self::SketchTool(_)
            | Self::ShapeMode(_)
            | Self::Constraint(_) => Scope::Sketch,
            Self::TogglePrincipal | Self::CopyFeatures | Self::PasteFeatures => {
                Scope::OutsideSketch
            }
            _ => Scope::Anywhere,
        }
    }

    pub fn default_shortcuts(self) -> Vec<KeyboardShortcut> {
        let command = |key| KeyboardShortcut::new(Modifiers::COMMAND, key);
        let command_shift = |key| KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, key);
        let plain = |key| KeyboardShortcut::new(Modifiers::NONE, key);
        let alt = |key| KeyboardShortcut::new(Modifiers::ALT, key);
        let alt_shift = |key| KeyboardShortcut::new(Modifiers::ALT | Modifiers::SHIFT, key);
        match self {
            Self::Palette => vec![command_shift(Key::P)],
            Self::New => vec![command(Key::N)],
            Self::Open => vec![command(Key::O)],
            Self::Save => vec![command(Key::S)],
            Self::SaveAs => vec![command_shift(Key::S)],
            Self::Import => vec![command(Key::I)],
            Self::Export => vec![command(Key::E)],
            Self::ExportImage => vec![command_shift(Key::E)],
            Self::Preferences => vec![command(Key::Comma)],
            Self::Quit => vec![command(Key::Q)],
            Self::Undo => vec![command(Key::Z)],
            Self::Redo => vec![command_shift(Key::Z), command(Key::Y)],
            Self::FitView => vec![plain(Key::F)],
            Self::Measure => vec![plain(Key::I)],
            Self::ToggleProjection => vec![plain(Key::O)],
            Self::LargerInterface => vec![command(Key::Plus), command(Key::Equals)],
            Self::SmallerInterface => vec![command(Key::Minus)],
            Self::NormalInterface => vec![command(Key::Num0)],
            Self::View(view) => vec![KeyboardShortcut::new(Modifiers::ALT, view.key())],
            Self::Camera(camera) => vec![camera.shortcut()],
            Self::HighlightNext => vec![plain(Key::N)],
            Self::HighlightPrevious => vec![KeyboardShortcut::new(Modifiers::SHIFT, Key::N)],
            Self::ActivateHighlighted => vec![plain(Key::Space)],
            Self::ListUnderPointer => vec![alt(Key::W)],
            Self::HideSelection => vec![plain(Key::H)],
            Self::HideOthers => vec![alt_shift(Key::H)],
            Self::LookAtFace => vec![alt(Key::V)],
            Self::ToggleGlyphs => vec![alt(Key::G)],
            Self::LookAtSketch => vec![alt_shift(Key::V)],
            Self::SelectAllShapes => vec![command_shift(Key::A)],
            Self::SelectTangentEdges => vec![alt(Key::T)],
            Self::SelectTangentFaces => vec![alt_shift(Key::T)],
            Self::SelectFaceEdges => vec![alt_shift(Key::E)],
            Self::ShowAll => vec![KeyboardShortcut::new(Modifiers::ALT, Key::H)],
            Self::SketchTool(tool) => tool_shortcut(tool).into_iter().collect(),
            Self::Constraint(tool) => vec![KeyboardShortcut::new(
                Modifiers::SHIFT,
                constraint_key(tool),
            )],
            Self::DeleteSelection => vec![plain(Key::Delete), plain(Key::Backspace)],
            Self::ReverseArc => vec![plain(Key::X)],
            Self::MoreSides => vec![plain(Key::CloseBracket)],
            Self::FewerSides => vec![plain(Key::OpenBracket)],
            Self::Construction => vec![plain(Key::Q)],
            Self::MoveGeometry => vec![plain(Key::M)],
            Self::SelectAll => vec![command(Key::A)],
            Self::CopyGeometry => vec![command(Key::C)],
            Self::CutGeometry => vec![command(Key::X)],
            Self::PasteGeometry => vec![command(Key::V)],
            Self::CopyFeatures => vec![command(Key::C)],
            Self::PasteFeatures => vec![command(Key::V)],
            Self::IntersectBody => vec![KeyboardShortcut::new(Modifiers::SHIFT, Key::Space)],
            Self::RenameFeature => vec![plain(Key::F2)],
            Self::GroupFeatures => vec![command(Key::G)],
            Self::Recompute => vec![plain(Key::F5)],
            Self::EditFeature => vec![plain(Key::E)],
            Self::ShowFirstFailed => vec![plain(Key::F8)],
            Self::FilterFeatures => vec![command(Key::F)],
            Self::RollbackUp => vec![KeyboardShortcut::new(Modifiers::ALT, Key::ArrowUp)],
            Self::RollbackDown => vec![KeyboardShortcut::new(Modifiers::ALT, Key::ArrowDown)],
            Self::FullScreen => vec![plain(Key::F11)],
            Self::NewSketch => vec![alt(Key::N)],
            Self::Extrude => vec![alt(Key::E)],
            Self::Revolve => vec![alt(Key::R)],
            Self::Hole => vec![alt(Key::O)],
            Self::NewBox => vec![alt(Key::B)],
            Self::NewCylinder => vec![alt(Key::Y)],
            Self::NewSphere => vec![alt(Key::U)],
            Self::NewTorus => vec![alt_shift(Key::U)],
            Self::Thread => vec![alt_shift(Key::O)],
            Self::Fillet => vec![alt(Key::F)],
            Self::Chamfer => vec![alt(Key::C)],
            Self::Shell => vec![alt(Key::S)],
            Self::OffsetFace => vec![alt(Key::Q)],
            Self::Combine => vec![alt(Key::J)],
            Self::Move => vec![alt(Key::M)],
            Self::CopyBody => vec![alt_shift(Key::C)],
            Self::Mirror => vec![alt_shift(Key::M)],
            Self::Split => vec![alt(Key::K)],
            Self::Scale => vec![alt_shift(Key::S)],
            Self::LinearPattern => vec![alt(Key::L)],
            Self::CircularPattern => vec![alt_shift(Key::L)],
            Self::DatumPlane => vec![alt(Key::D)],
            Self::DatumAxis => vec![alt_shift(Key::D)],
            Self::DatumPoint => vec![alt_shift(Key::P)],
            Self::ClearChosenRegions => vec![alt_shift(Key::R)],
            Self::TogglePrincipal => vec![plain(Key::P)],
            Self::VersionHistory
            | Self::CoordinateSystem
            | Self::ScaleModel
            | Self::ModelProperties
            | Self::KeyboardShortcuts
            | Self::ExportSketch
            | Self::ExportFace
            | Self::KeepDrawingConstruction
            | Self::Interference
            | Self::BodyAppearance
            | Self::RenameBody
            | Self::RemoveBody
            | Self::SelectBody
            | Self::SelectHole
            | Self::SaveSelectionSet
            | Self::SelectionSets
            | Self::FinishSketch
            | Self::ShapeMode(_)
            | Self::ToggleConstraintActive
            | Self::SplitCurve
            | Self::BreakCurves
            | Self::RotateGeometry
            | Self::ScaleGeometry
            | Self::ToggleTypedDimensions
            | Self::ToggleFirstDimensionScales
            | Self::Filter(_)
            | Self::Style(_)
            | Self::Analysis(_)
            | Self::NewFromTemplate
            | Self::SaveAsTemplate
            | Self::ImportParameters
            | Self::ExportParameters
            | Self::OpenSample(_)
            | Self::OpenRecent(_)
            | Self::ClearRecent
            | Self::RecoverUnsaved
            | Self::CancelExport
            | Self::CancelImageExport
            | Self::CancelImport
            | Self::CancelRecompute
            | Self::Ungroup
            | Self::RenameGroup
            | Self::MoveFeatureUp
            | Self::MoveFeatureDown
            | Self::DeleteFeature
            | Self::SuppressFeature
            | Self::RollToHere
            | Self::RollToEnd
            | Self::ToggleVisibility
            | Self::ToggleSnapping
            | Self::ToggleGridSnapping
            | Self::AutomaticProjection
            | Self::ToggleLasso
            | Self::TogglePaintSelection
            | Self::ToggleSelectThrough
            | Self::CycleSelectionPriority
            | Self::ToggleCentresOfMass
            | Self::MinimizeWindow
            | Self::MaximizeWindow
            | Self::SaveView
            | Self::SavedViews
            | Self::SetHomeView
            | Self::ResetHomeView
            | Self::CloseFeature
            | Self::DetachSketch
            | Self::PlaceSketch
            | Self::UseSelectedAxis
            | Self::ExtrudeUpToSelected
            | Self::StartAtSelected
            | Self::MirrorAcrossSelected
            | Self::SplitAlongSelected
            | Self::DatumUseSelected
            | Self::DatumTurnAboutSelected
            | Self::PatternUseSelected
            | Self::PatternSecondUseSelected
            | Self::MoveTurnAboutSelected
            | Self::AddParameter
            | Self::DeleteParameter
            | Self::MoveParameterUp
            | Self::MoveParameterDown
            | Self::ParameterNote
            | Self::UpdateReferences
            | Self::ReplaceImport
            | Self::ReloadImport
            | Self::DismissNotice
            | Self::DismissTip
            | Self::HideTips
            | Self::Welcome
            | Self::About
            | Self::Messages
            | Self::Mate
            | Self::UndoHistory
            | Self::SelectFree
            | Self::ToggleSketches
            | Self::ToggleDatums
            | Self::ToggleBodies
            | Self::NewCone
            | Self::NewWedge
            | Self::NewPrism => Vec::new(),
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        Self::all().find(|command| command.id() == id)
    }
}

fn tool_shortcut(tool: Tool) -> Option<KeyboardShortcut> {
    let plain = |key| Some(KeyboardShortcut::new(Modifiers::NONE, key));
    match tool {
        Tool::Select => None,
        Tool::Point => plain(Key::P),
        Tool::Line => plain(Key::L),
        Tool::Rectangle => plain(Key::R),
        Tool::Circle => plain(Key::C),
        Tool::Arc => plain(Key::A),
        Tool::ThreePointArc => Some(KeyboardShortcut::new(Modifiers::ALT, Key::A)),
        Tool::TangentArc => plain(Key::T),
        Tool::Slot => plain(Key::U),
        Tool::Polygon => plain(Key::G),
        Tool::Spline => plain(Key::S),
        Tool::Trim => plain(Key::K),
        Tool::Extend => plain(Key::J),
        Tool::Offset => plain(Key::W),
        Tool::Mirror => plain(Key::Y),
        Tool::Fillet => plain(Key::B),
        Tool::Chamfer => Some(KeyboardShortcut::new(Modifiers::SHIFT, Key::B)),
        Tool::RectangularPattern
        | Tool::CircularPattern
        | Tool::TangentCircle
        | Tool::Ellipse
        | Tool::EllipticalArc => None,
        Tool::Project => Some(KeyboardShortcut::new(Modifiers::ALT, Key::P)),
        Tool::Intersect => Some(KeyboardShortcut::new(Modifiers::ALT, Key::I)),
        Tool::Dimension => plain(Key::D),
        Tool::BlendCurve => Some(KeyboardShortcut::new(
            Modifiers::ALT | Modifiers::SHIFT,
            Key::B,
        )),
    }
}

fn constraint_key(tool: ConstraintTool) -> Key {
    match tool {
        ConstraintTool::Coincident => Key::C,
        ConstraintTool::Midpoint => Key::M,
        ConstraintTool::Concentric => Key::O,
        ConstraintTool::Collinear => Key::I,
        ConstraintTool::Fix => Key::F,
        ConstraintTool::Horizontal => Key::H,
        ConstraintTool::Vertical => Key::V,
        ConstraintTool::Parallel => Key::P,
        ConstraintTool::Perpendicular => Key::L,
        ConstraintTool::Tangent => Key::T,
        ConstraintTool::Curvature => Key::G,
        ConstraintTool::Equal => Key::E,
        ConstraintTool::Symmetric => Key::S,
        ConstraintTool::Distance => Key::D,
        ConstraintTool::HorizontalDistance => Key::X,
        ConstraintTool::VerticalDistance => Key::Y,
        ConstraintTool::Angle => Key::A,
        ConstraintTool::Radius => Key::R,
        ConstraintTool::Diameter => Key::W,
    }
}

pub fn normalized(modifiers: Modifiers) -> Modifiers {
    Modifiers {
        alt: modifiers.alt,
        ctrl: false,
        shift: modifiers.shift,
        mac_cmd: false,
        command: modifiers.command || modifiers.ctrl || modifiers.mac_cmd,
    }
}

pub fn is_reserved(key: Key) -> bool {
    RESERVED_KEYS.contains(&key)
}

pub fn is_modifier(key: Key) -> bool {
    MODIFIER_KEYS.contains(&key)
}

fn is_symbol(key: Key) -> bool {
    matches!(
        key,
        Key::Colon
            | Key::Comma
            | Key::Backslash
            | Key::Slash
            | Key::Pipe
            | Key::Questionmark
            | Key::Exclamationmark
            | Key::OpenBracket
            | Key::CloseBracket
            | Key::OpenCurlyBracket
            | Key::CloseCurlyBracket
            | Key::Backtick
            | Key::Minus
            | Key::Period
            | Key::Plus
            | Key::Equals
            | Key::Semicolon
            | Key::Quote
    )
}

pub fn stored_text(shortcut: &KeyboardShortcut) -> String {
    let modifiers = normalized(shortcut.modifiers);
    let names = [
        (modifiers.command, CTRL),
        (modifiers.alt, ALT),
        (modifiers.shift, SHIFT),
    ];
    names
        .into_iter()
        .filter_map(|(pressed, name)| pressed.then_some(name))
        .chain([shortcut.logical_key.name()])
        .collect::<Vec<_>>()
        .join("+")
}

pub fn display(shortcut: &KeyboardShortcut) -> String {
    let key = shortcut.logical_key;
    let key_text = if is_symbol(key) {
        key.symbol_or_name()
    } else {
        key.name()
    };
    let modifiers = normalized(shortcut.modifiers);
    [
        (modifiers.command, CTRL),
        (modifiers.alt, ALT),
        (modifiers.shift, SHIFT),
    ]
    .into_iter()
    .filter_map(|(pressed, name)| pressed.then_some(name))
    .chain([key_text])
    .collect::<Vec<_>>()
    .join("+")
}

pub fn is_named_by(shortcut: &KeyboardShortcut, query: &str) -> bool {
    let modifiers = normalized(shortcut.modifiers);
    let held = [
        (modifiers.command, CTRL),
        (modifiers.alt, ALT),
        (modifiers.shift, SHIFT),
    ];
    let key = shortcut.logical_key;
    let names_part = |part: &str| {
        part.eq_ignore_ascii_case(key.name())
            || part.eq_ignore_ascii_case(key.symbol_or_name())
            || held
                .iter()
                .any(|(pressed, name)| *pressed && part.eq_ignore_ascii_case(name))
    };
    let parts = query_keys(query);
    !parts.is_empty() && parts.into_iter().all(names_part)
}

fn query_keys(query: &str) -> Vec<&str> {
    let query = query.trim();
    let mut parts: Vec<&str> = query
        .split(|character: char| character == '+' || character.is_whitespace())
        .filter(|part| !part.is_empty())
        .collect();
    if query == "+" || query.ends_with("++") {
        parts.push("+");
    }
    parts
}

pub fn parse_stored(text: &str) -> Option<KeyboardShortcut> {
    let mut parts: Vec<&str> = text.split('+').map(str::trim).collect();
    let key = Key::from_name(parts.pop()?)?;
    if is_reserved(key) || is_modifier(key) {
        return None;
    }
    let mut modifiers = Modifiers::NONE;
    for part in parts {
        modifiers = modifiers.plus(match part {
            CTRL => Modifiers::COMMAND,
            ALT => Modifiers::ALT,
            SHIFT => Modifiers::SHIFT,
            _ => return None,
        });
    }
    Some(KeyboardShortcut::new(modifiers, key))
}

fn same(a: &KeyboardShortcut, b: &KeyboardShortcut) -> bool {
    a.logical_key == b.logical_key && normalized(a.modifiers) == normalized(b.modifiers)
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Keymap {
    overrides: BTreeMap<Command, Vec<KeyboardShortcut>>,
}

impl Keymap {
    pub fn from_settings(settings: &Settings) -> Self {
        let mut keymap = Self::default();
        for key in settings.keys_under(SETTINGS_PREFIX) {
            let Some(command) = key.strip_prefix(SETTINGS_PREFIX).and_then(Command::from_id) else {
                continue;
            };
            let Some(texts) = settings.texts(&key) else {
                continue;
            };
            let shortcuts: Vec<KeyboardShortcut> =
                texts.iter().filter_map(|text| parse_stored(text)).collect();
            if shortcuts.is_empty() && !texts.is_empty() {
                continue;
            }
            keymap.set(command, shortcuts);
        }
        keymap
    }

    pub fn write(&self, loaded: &Self, settings: &mut Settings) {
        for command in Command::all() {
            let shortcuts = self.shortcuts(command);
            if shortcuts == loaded.shortcuts(command) {
                continue;
            }
            let key = format!("{SETTINGS_PREFIX}{}", command.id());
            if self.is_default(command) {
                settings.remove(&key);
            } else {
                let texts: Vec<String> = shortcuts.iter().map(stored_text).collect();
                settings.set_texts(&key, &texts);
            }
        }
    }

    pub fn shortcuts(&self, command: Command) -> Vec<KeyboardShortcut> {
        self.overrides
            .get(&command)
            .cloned()
            .unwrap_or_else(|| command.default_shortcuts())
    }

    pub fn first(&self, command: Command) -> Option<KeyboardShortcut> {
        self.shortcuts(command).first().copied()
    }

    pub fn is_default(&self, command: Command) -> bool {
        !self.overrides.contains_key(&command)
    }

    pub fn is_all_default(&self) -> bool {
        self.overrides.is_empty()
    }

    pub fn conflicts(&self, command: Command, shortcut: &KeyboardShortcut) -> Vec<Command> {
        Command::all()
            .filter(|other| *other != command && other.scope().overlaps(command.scope()))
            .filter(|other| {
                self.shortcuts(*other)
                    .iter()
                    .any(|bound| same(bound, shortcut))
            })
            .collect()
    }

    pub fn bind(&mut self, command: Command, shortcut: KeyboardShortcut) {
        for other in self.conflicts(command, &shortcut) {
            self.unbind(other, shortcut);
        }
        let mut shortcuts = self.shortcuts(command);
        if !shortcuts.iter().any(|bound| same(bound, &shortcut)) {
            shortcuts.push(shortcut);
        }
        self.set(command, shortcuts);
    }

    pub fn unbind(&mut self, command: Command, shortcut: KeyboardShortcut) {
        let mut shortcuts = self.shortcuts(command);
        shortcuts.retain(|bound| !same(bound, &shortcut));
        self.set(command, shortcuts);
    }

    pub fn reset(&mut self, command: Command) {
        self.overrides.remove(&command);
        for shortcut in command.default_shortcuts() {
            for other in self.conflicts(command, &shortcut) {
                self.unbind(other, shortcut);
            }
        }
    }

    pub fn reset_all(&mut self) {
        self.overrides.clear();
    }

    fn set(&mut self, command: Command, shortcuts: Vec<KeyboardShortcut>) {
        if shortcuts == command.default_shortcuts() {
            self.overrides.remove(&command);
        } else {
            self.overrides.insert(command, shortcuts);
        }
    }

    fn command_for(
        &self,
        pressed: &KeyboardShortcut,
        situation: &Situation,
    ) -> Option<(Command, KeyboardShortcut)> {
        let candidates: Vec<(Command, KeyboardShortcut)> = Command::all()
            .filter(|command| situation.allows(*command))
            .flat_map(|command| {
                self.shortcuts(command)
                    .into_iter()
                    .map(move |shortcut| (command, shortcut))
            })
            .filter(|(_, shortcut)| shortcut.logical_key == pressed.logical_key)
            .collect();
        let pressed_modifiers = normalized(pressed.modifiers);
        let specific_first = |(command, _): &(Command, KeyboardShortcut)| {
            u8::from(command.scope() == Scope::Anywhere)
        };
        let mut exact: Vec<_> = candidates
            .iter()
            .filter(|(_, shortcut)| normalized(shortcut.modifiers) == pressed_modifiers)
            .copied()
            .collect();
        exact.sort_by_key(specific_first);
        if let Some(found) = exact.first() {
            return Some(*found);
        }
        if !is_symbol(pressed.logical_key) {
            return None;
        }
        candidates
            .into_iter()
            .filter(|(_, shortcut)| pressed_modifiers.matches_logically(shortcut.modifiers))
            .max_by_key(|(_, shortcut)| {
                let modifiers = normalized(shortcut.modifiers);
                u8::from(modifiers.shift) + u8::from(modifiers.alt)
            })
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Situation {
    pub editing_sketch: bool,
    pub drawing: bool,
    pub text_focused: bool,
    pub keys_free: bool,
}

impl Situation {
    fn allows(&self, command: Command) -> bool {
        match command.scope() {
            Scope::Anywhere => true,
            Scope::Sketch => self.editing_sketch,
            Scope::OutsideSketch => !self.editing_sketch,
        }
    }

    fn accepts(&self, shortcut: &KeyboardShortcut) -> bool {
        let modifiers = normalized(shortcut.modifiers);
        if self.drawing && KEPT_WHILE_DRAWING.contains(&shortcut.logical_key) {
            return false;
        }
        if modifiers.command || modifiers.alt {
            !self.text_focused || !TEXT_EDITING_KEYS.contains(&shortcut.logical_key)
        } else if WIDGET_KEYS.contains(&shortcut.logical_key) {
            self.keys_free
        } else {
            self.keys_free || !self.text_focused
        }
    }
}

pub fn pressed_shortcut(event: &Event) -> Option<KeyboardShortcut> {
    match event {
        Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } if !is_modifier(*key) => Some(KeyboardShortcut::new(normalized(*modifiers), *key)),
        Event::Copy => Some(KeyboardShortcut::new(Modifiers::COMMAND, Key::C)),
        Event::Cut => Some(KeyboardShortcut::new(Modifiers::COMMAND, Key::X)),
        Event::Paste(_) => Some(KeyboardShortcut::new(Modifiers::COMMAND, Key::V)),
        _ => None,
    }
}

pub fn dispatch(ctx: &egui::Context, keymap: &Keymap, situation: &Situation) -> Vec<Command> {
    let mut triggered = Vec::new();
    ctx.input_mut(|input| {
        input.events.retain(|event| {
            let Some(pressed) = pressed_shortcut(event) else {
                return true;
            };
            let repeated = matches!(event, Event::Key { repeat: true, .. });
            let found = keymap
                .command_for(&pressed, situation)
                .filter(|(_, shortcut)| situation.accepts(shortcut));
            match found {
                Some((command, _)) => {
                    if !repeated || command.repeats() {
                        triggered.push(command);
                    }
                    false
                }
                None => true,
            }
        });
    });
    triggered
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub command: Command,
    pub availability: Result<(), String>,
    pub detail: Option<String>,
}

impl Offer {
    pub fn title(&self) -> String {
        match &self.detail {
            Some(detail) => format!("{}: {detail}", self.command.title()),
            None => self.command.title(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Clipboard {
    #[default]
    Unread,
    Read(Option<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pasted<'a> {
    Unread,
    Text(&'a str),
    Nothing,
}

pub struct CommandFrame<'a> {
    keymap: &'a Keymap,
    triggered: Vec<Command>,
    offers: Vec<Offer>,
    refused: Vec<(Command, String)>,
    clipboard: Clipboard,
    paste_asked: Option<Command>,
    copied: Option<String>,
}

impl<'a> CommandFrame<'a> {
    pub fn new(keymap: &'a Keymap, triggered: Vec<Command>) -> Self {
        Self {
            keymap,
            triggered,
            offers: Vec::new(),
            refused: Vec::new(),
            clipboard: Clipboard::Unread,
            paste_asked: None,
            copied: None,
        }
    }

    #[must_use]
    pub fn with_clipboard(mut self, clipboard: Clipboard) -> Self {
        self.clipboard = clipboard;
        self
    }

    pub fn pasted(&self) -> Pasted<'_> {
        match &self.clipboard {
            Clipboard::Unread => Pasted::Unread,
            Clipboard::Read(Some(text)) => Pasted::Text(text),
            Clipboard::Read(None) => Pasted::Nothing,
        }
    }

    pub fn ask_for_paste(&mut self, command: Command) {
        self.paste_asked = Some(command);
    }

    pub fn paste_asked(&self) -> Option<Command> {
        self.paste_asked
    }

    pub fn copy(&mut self, text: String) {
        self.copied = Some(text);
    }

    pub fn take_copied(&mut self) -> Option<String> {
        self.copied.take()
    }

    pub fn keys(&self, command: Command) -> Option<String> {
        self.keymap
            .first(command)
            .map(|shortcut| display(&shortcut))
    }

    pub fn with_keys(&self, command: Command, text: &str) -> String {
        match self.keys(command) {
            Some(keys) => format!("{text} ({keys})"),
            None => text.to_owned(),
        }
    }

    pub fn invoke<T, E: ToString>(
        &mut self,
        command: Command,
        availability: &Result<T, E>,
    ) -> bool {
        self.invoke_detailed(command, None, availability)
    }

    pub fn invoke_detailed<T, E: ToString>(
        &mut self,
        command: Command,
        detail: Option<String>,
        availability: &Result<T, E>,
    ) -> bool {
        let availability = availability
            .as_ref()
            .map(|_| ())
            .map_err(ToString::to_string);
        let triggered = self.take(command);
        if triggered && let Err(reason) = &availability {
            self.refused.push((command, reason.clone()));
        }
        self.offers.retain(|offer| offer.command != command);
        let ready = availability.is_ok();
        self.offers.push(Offer {
            command,
            availability,
            detail,
        });
        triggered && ready
    }

    pub fn available(&mut self, command: Command) -> bool {
        self.invoke(command, &Ok::<(), String>(()))
    }

    pub fn trigger(&mut self, command: Command) {
        self.triggered.push(command);
    }

    pub fn take(&mut self, command: Command) -> bool {
        let before = self.triggered.len();
        self.triggered.retain(|triggered| *triggered != command);
        self.triggered.len() != before
    }

    pub fn offers(&self) -> &[Offer] {
        &self.offers
    }

    pub fn finish(self) -> (Vec<Offer>, Vec<(Command, String)>) {
        (self.offers, self.refused)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(key: Key, modifiers: Modifiers) -> KeyboardShortcut {
        KeyboardShortcut::new(modifiers, key)
    }

    #[test]
    fn every_command_has_a_distinct_id_and_the_defaults_do_not_collide() {
        let commands: Vec<Command> = Command::all().collect();
        for command in &commands {
            assert_eq!(Command::from_id(command.id()), Some(*command));
        }
        let keymap = Keymap::default();
        for command in commands {
            for shortcut in keymap.shortcuts(command) {
                assert!(
                    keymap.conflicts(command, &shortcut).is_empty(),
                    "{} collides",
                    command.id()
                );
                assert_eq!(parse_stored(&stored_text(&shortcut)), Some(shortcut));
            }
        }
    }

    #[test]
    fn shortcuts_are_found_by_any_of_their_keys_in_any_order_and_case() {
        let redo = press(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT);
        let zoom_in = press(Key::Plus, Modifiers::COMMAND);

        for query in ["Ctrl+Shift+Z", "shift ctrl z", "z", "ctrl+", " SHIFT "] {
            assert!(is_named_by(&redo, query), "{query}");
        }
        for query in ["", "+", "alt+z", "s", "ctrl+y"] {
            assert!(!is_named_by(&redo, query), "{query}");
        }
        for query in ["Ctrl++", "ctrl plus", "+"] {
            assert!(is_named_by(&zoom_in, query), "{query}");
        }
    }

    #[test]
    fn plain_keys_that_widgets_ignore_work_while_a_button_has_focus() {
        let button_focused = Situation {
            keys_free: false,
            ..Situation::default()
        };
        let text_focused = Situation {
            text_focused: true,
            ..button_focused
        };
        let delete = press(Key::Delete, Modifiers::NONE);
        let space = press(Key::Space, Modifiers::NONE);
        assert!(button_focused.accepts(&delete));
        assert!(!button_focused.accepts(&space));
        assert!(!text_focused.accepts(&delete));
    }

    #[test]
    fn stored_shortcuts_read_back_and_reserved_keys_are_refused() {
        assert_eq!(
            parse_stored("Ctrl+Shift+Z"),
            Some(press(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT))
        );
        assert_eq!(parse_stored("Alt+F4"), Some(press(Key::F4, Modifiers::ALT)));
        assert_eq!(
            parse_stored("Ctrl+Plus"),
            Some(press(Key::Plus, Modifiers::COMMAND))
        );
        assert_eq!(parse_stored("Escape"), None);
        assert_eq!(parse_stored("Hyper+Q"), None);
        assert_eq!(parse_stored("Ctrl+Nothing"), None);
        let comma = press(Key::Comma, Modifiers::CTRL | Modifiers::COMMAND);
        assert_eq!(stored_text(&comma), "Ctrl+Comma");
        assert_eq!(display(&comma), "Ctrl+,");
        assert_eq!(
            display(&press(Key::Z, Modifiers::SHIFT | Modifiers::COMMAND)),
            "Ctrl+Shift+Z"
        );
    }

    #[test]
    fn binding_a_taken_shortcut_moves_it_and_reset_takes_it_back() {
        let mut keymap = Keymap::default();
        let f = press(Key::F, Modifiers::NONE);
        let line = Command::SketchTool(Tool::Line);
        assert_eq!(keymap.conflicts(line, &f), vec![Command::FitView]);
        keymap.bind(line, f);
        assert!(keymap.shortcuts(Command::FitView).is_empty());
        assert_eq!(
            keymap.shortcuts(line),
            vec![press(Key::L, Modifiers::NONE), f]
        );
        keymap.reset(Command::FitView);
        assert_eq!(keymap.shortcuts(Command::FitView), vec![f]);
        assert_eq!(keymap.shortcuts(line), vec![press(Key::L, Modifiers::NONE)]);
        assert!(keymap.is_all_default());

        let tool = Command::SketchTool(Tool::Point);
        let horizontal = Command::Constraint(ConstraintTool::Horizontal);
        assert_eq!(
            keymap.conflicts(Command::Extrude, &press(Key::H, Modifiers::SHIFT)),
            vec![horizontal]
        );
        assert!(
            keymap
                .conflicts(tool, &press(Key::P, Modifiers::COMMAND))
                .is_empty()
        );
    }

    #[test]
    fn only_changed_bindings_are_written_and_unknown_ones_are_kept() {
        let mut raw = Settings::default();
        raw.set_texts("keys.file.save", &["Ctrl+Nothing".to_owned()]);
        raw.set_texts("keys.future.command", &["Ctrl+K".to_owned()]);
        raw.set_texts("keys.file.open", &[]);
        let loaded = Keymap::from_settings(&raw);
        assert_eq!(
            loaded.shortcuts(Command::Save),
            Command::Save.default_shortcuts()
        );
        assert!(loaded.shortcuts(Command::Open).is_empty());
        let mut keymap = loaded.clone();
        keymap.bind(Command::Extrude, press(Key::E, Modifiers::NONE));
        let mut settings = raw.clone();
        keymap.write(&loaded, &mut settings);
        assert_eq!(
            settings.texts("keys.file.save"),
            Some(vec!["Ctrl+Nothing".to_owned()])
        );
        assert_eq!(
            settings.texts("keys.future.command"),
            Some(vec!["Ctrl+K".to_owned()])
        );
        assert_eq!(
            settings.texts("keys.model.extrude"),
            Some(vec!["Alt+E".to_owned(), "E".to_owned()])
        );
        keymap.bind(Command::Save, press(Key::F2, Modifiers::COMMAND));
        keymap.write(&loaded, &mut settings);
        assert_eq!(
            settings.texts("keys.file.save"),
            Some(vec!["Ctrl+S".to_owned(), "Ctrl+F2".to_owned()])
        );
        let rebound = keymap.clone();
        keymap.reset(Command::Save);
        keymap.write(&rebound, &mut settings);
        assert_eq!(settings.texts("keys.file.save"), None);
        assert_eq!(Keymap::from_settings(&settings), keymap);
    }

    #[test]
    fn a_press_finds_its_exact_binding_before_a_looser_one() {
        let keymap = Keymap::default();
        let sketch = Situation {
            editing_sketch: true,
            keys_free: true,
            ..Situation::default()
        };
        let found = |pressed, situation: &Situation| {
            keymap
                .command_for(&pressed, situation)
                .map(|(command, _)| command)
        };
        assert_eq!(
            found(press(Key::P, Modifiers::SHIFT), &sketch),
            Some(Command::Constraint(ConstraintTool::Parallel))
        );
        assert_eq!(
            found(press(Key::P, Modifiers::NONE), &sketch),
            Some(Command::SketchTool(Tool::Point))
        );
        assert_eq!(
            found(press(Key::P, Modifiers::NONE), &Situation::default()),
            Some(Command::TogglePrincipal)
        );
        assert_eq!(
            found(
                press(Key::S, Modifiers::COMMAND | Modifiers::SHIFT),
                &sketch
            ),
            Some(Command::SaveAs)
        );
        assert_eq!(
            found(press(Key::S, Modifiers::ALT), &sketch),
            Some(Command::Shell)
        );
        assert_eq!(found(press(Key::Z, Modifiers::ALT), &sketch), None);
        assert_eq!(
            found(
                press(Key::Comma, Modifiers::COMMAND | Modifiers::SHIFT),
                &sketch
            ),
            Some(Command::Preferences)
        );
        assert!(sketch.accepts(&press(Key::Delete, Modifiers::NONE)));
        let drawing = Situation {
            drawing: true,
            ..sketch
        };
        assert!(!drawing.accepts(&press(Key::Backspace, Modifiers::NONE)));
        let typing = Situation {
            text_focused: true,
            ..Situation::default()
        };
        assert!(typing.accepts(&press(Key::S, Modifiers::COMMAND)));
        assert!(typing.accepts(&press(Key::Num1, Modifiers::ALT)));
        for kept in [Key::C, Key::V, Key::Z, Key::ArrowLeft, Key::Backspace] {
            assert!(
                !typing.accepts(&press(kept, Modifiers::COMMAND)),
                "{kept:?}"
            );
        }
        assert!(!typing.accepts(&press(Key::S, Modifiers::NONE)));
    }
}
