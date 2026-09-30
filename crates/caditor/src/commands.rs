use std::collections::BTreeMap;

use caditor_file::Settings;
use caditor_geometry::Vector3;
use egui::{Event, Key, KeyboardShortcut, Modifiers};

use crate::{editing::Tool, samples::Sample, sketch_tools::ConstraintTool};

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
    Open,
    Save,
    SaveAs,
    VersionHistory,
    Import,
    Export,
    Preferences,
    KeyboardShortcuts,
    Quit,
    Undo,
    Redo,
    NewSketch,
    FinishSketch,
    ReverseArc,
    Construction,
    MoveGeometry,
    SelectAll,
    SketchTool(Tool),
    Constraint(ConstraintTool),
    DeleteSelection,
    Extrude,
    Revolve,
    Fillet,
    Chamfer,
    Shell,
    DatumPlane,
    DatumAxis,
    FitView,
    LargerInterface,
    SmallerInterface,
    NormalInterface,
    View(StandardView),
    Camera(CameraMove),
    HighlightNext,
    HighlightPrevious,
    ActivateHighlighted,
    HideSelection,
    ToggleVisibility,
    ShowAll,
    TogglePrincipal,
    ToggleProjection,
    OpenSample(Sample),
    OpenRecent(RecentSlot),
    RecoverUnsaved,
    CancelExport,
    Recompute,
    CancelRecompute,
    RenameFeature,
    MoveFeatureUp,
    MoveFeatureDown,
    DeleteFeature,
    EditFeature,
    CloseFeature,
    DetachSketch,
    PlaceSketch,
    UseSelectedAxis,
    DatumUseSelected,
    DatumTurnAboutSelected,
    AddParameter,
    DeleteParameter,
    ShowFirstFailed,
    DismissNotice,
    DismissTip,
    HideTips,
    Welcome,
    About,
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

impl StandardView {
    pub const ALL: [Self; 7] = [
        Self::Isometric,
        Self::Front,
        Self::Top,
        Self::Right,
        Self::Back,
        Self::Bottom,
        Self::Left,
    ];

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

impl CameraMove {
    pub const ALL: [Self; 10] = [
        Self::OrbitLeft,
        Self::OrbitRight,
        Self::OrbitUp,
        Self::OrbitDown,
        Self::PanLeft,
        Self::PanRight,
        Self::PanUp,
        Self::PanDown,
        Self::ZoomIn,
        Self::ZoomOut,
    ];

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
}

impl Scope {
    fn overlaps(self, other: Self) -> bool {
        self == Self::Anywhere || other == Self::Anywhere || self == other
    }

    pub fn describe(self) -> &'static str {
        match self {
            Self::Anywhere => "anywhere",
            Self::Sketch => "while editing a sketch",
        }
    }
}

const PLAIN_COMMANDS: [Command; 62] = [
    Command::Palette,
    Command::New,
    Command::Open,
    Command::Save,
    Command::SaveAs,
    Command::VersionHistory,
    Command::Import,
    Command::Export,
    Command::Preferences,
    Command::KeyboardShortcuts,
    Command::Welcome,
    Command::About,
    Command::Quit,
    Command::Undo,
    Command::Redo,
    Command::FitView,
    Command::ToggleProjection,
    Command::LargerInterface,
    Command::SmallerInterface,
    Command::NormalInterface,
    Command::HighlightNext,
    Command::HighlightPrevious,
    Command::ActivateHighlighted,
    Command::HideSelection,
    Command::ToggleVisibility,
    Command::ShowAll,
    Command::TogglePrincipal,
    Command::NewSketch,
    Command::Extrude,
    Command::Revolve,
    Command::Fillet,
    Command::Chamfer,
    Command::Shell,
    Command::DatumPlane,
    Command::DatumAxis,
    Command::FinishSketch,
    Command::ReverseArc,
    Command::Construction,
    Command::MoveGeometry,
    Command::SelectAll,
    Command::DeleteSelection,
    Command::RecoverUnsaved,
    Command::CancelExport,
    Command::Recompute,
    Command::CancelRecompute,
    Command::RenameFeature,
    Command::MoveFeatureUp,
    Command::MoveFeatureDown,
    Command::DeleteFeature,
    Command::EditFeature,
    Command::CloseFeature,
    Command::DetachSketch,
    Command::PlaceSketch,
    Command::UseSelectedAxis,
    Command::DatumUseSelected,
    Command::DatumTurnAboutSelected,
    Command::AddParameter,
    Command::DeleteParameter,
    Command::ShowFirstFailed,
    Command::DismissNotice,
    Command::DismissTip,
    Command::HideTips,
];

impl Command {
    pub fn all() -> impl Iterator<Item = Self> {
        PLAIN_COMMANDS
            .into_iter()
            .chain(Tool::ALL.into_iter().map(Self::SketchTool))
            .chain(ConstraintTool::ALL.into_iter().map(Self::Constraint))
            .chain(StandardView::ALL.into_iter().map(Self::View))
            .chain(CameraMove::ALL.into_iter().map(Self::Camera))
            .chain(Sample::ALL.into_iter().map(Self::OpenSample))
            .chain(RecentSlot::ALL.into_iter().map(Self::OpenRecent))
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Palette => "palette",
            Self::New => "file.new",
            Self::Open => "file.open",
            Self::Save => "file.save",
            Self::SaveAs => "file.save_as",
            Self::VersionHistory => "file.history",
            Self::Import => "file.import",
            Self::Export => "file.export",
            Self::Preferences => "file.preferences",
            Self::KeyboardShortcuts => "file.shortcuts",
            Self::Quit => "file.quit",
            Self::Undo => "edit.undo",
            Self::Redo => "edit.redo",
            Self::NewSketch => "model.new_sketch",
            Self::FinishSketch => "sketch.finish",
            Self::ReverseArc => "sketch.reverse_arc",
            Self::Construction => "sketch.construction",
            Self::MoveGeometry => "sketch.move",
            Self::SelectAll => "sketch.select_all",
            Self::SketchTool(tool) => match tool {
                Tool::Select => "sketch.select",
                Tool::Point => "sketch.point",
                Tool::Line => "sketch.line",
                Tool::Rectangle => "sketch.rectangle",
                Tool::Circle => "sketch.circle",
                Tool::Arc => "sketch.arc",
                Tool::Spline => "sketch.spline",
            },
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
            Self::Fillet => "model.fillet",
            Self::Chamfer => "model.chamfer",
            Self::Shell => "model.shell",
            Self::DatumPlane => "model.plane",
            Self::DatumAxis => "model.axis",
            Self::FitView => "view.fit",
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
            Self::HideSelection => "view.hide_selection",
            Self::ToggleVisibility => "view.toggle_visibility",
            Self::ShowAll => "view.show_all",
            Self::TogglePrincipal => "view.toggle_principal",
            Self::ToggleProjection => "view.toggle_projection",
            Self::OpenSample(sample) => match sample {
                Sample::Plate => "file.sample.plate",
                Sample::Spool => "file.sample.spool",
                Sample::Bracket => "file.sample.bracket",
            },
            Self::OpenRecent(slot) => slot.id(),
            Self::RecoverUnsaved => "file.recover",
            Self::CancelExport => "file.cancel_export",
            Self::Recompute => "model.recompute",
            Self::CancelRecompute => "model.cancel_recompute",
            Self::RenameFeature => "model.rename_feature",
            Self::MoveFeatureUp => "model.move_feature_up",
            Self::MoveFeatureDown => "model.move_feature_down",
            Self::DeleteFeature => "model.delete_feature",
            Self::EditFeature => "model.edit_feature",
            Self::CloseFeature => "model.close_feature",
            Self::DetachSketch => "model.detach_sketch",
            Self::PlaceSketch => "model.place_sketch",
            Self::UseSelectedAxis => "model.use_selected_axis",
            Self::DatumUseSelected => "model.datum_use_selected",
            Self::DatumTurnAboutSelected => "model.datum_turn_about_selected",
            Self::AddParameter => "model.add_parameter",
            Self::DeleteParameter => "model.delete_parameter",
            Self::ShowFirstFailed => "model.first_failed",
            Self::DismissNotice => "edit.dismiss_notice",
            Self::DismissTip => "help.dismiss_tip",
            Self::HideTips => "help.hide_tips",
            Self::Welcome => "help.welcome",
            Self::About => "help.about",
        }
    }

    pub fn title(self) -> String {
        let fixed = match self {
            Self::Palette => "Find a command",
            Self::New => "New model",
            Self::Open => "Open…",
            Self::Save => "Save",
            Self::SaveAs => "Save As…",
            Self::VersionHistory => "Version History…",
            Self::Import => "Import…",
            Self::Export => "Export…",
            Self::Preferences => "Preferences…",
            Self::KeyboardShortcuts => "Keyboard Shortcuts…",
            Self::Quit => "Quit",
            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::NewSketch => "New sketch",
            Self::FinishSketch => "Finish sketch",
            Self::ReverseArc => "Reverse the arc",
            Self::Construction => "Switch to or from construction geometry",
            Self::MoveGeometry => "Move selected sketch geometry",
            Self::SelectAll => "Select all sketch geometry",
            Self::SketchTool(Tool::Select) => "Select tool",
            Self::SketchTool(tool) => return format!("Draw {}", tool.label().to_lowercase()),
            Self::Constraint(tool) => return tool.label().to_owned(),
            Self::DeleteSelection => "Delete selection",
            Self::Extrude => "Extrude",
            Self::Revolve => "Revolve",
            Self::Fillet => "Fillet",
            Self::Chamfer => "Chamfer",
            Self::Shell => "Shell",
            Self::DatumPlane => "Datum plane",
            Self::DatumAxis => "Datum axis",
            Self::FitView => "Fit view",
            Self::LargerInterface => "Make the interface larger",
            Self::SmallerInterface => "Make the interface smaller",
            Self::NormalInterface => "Interface at normal size",
            Self::View(StandardView::Isometric) => "Isometric view",
            Self::View(view) => return format!("View from the {}", view.name()),
            Self::Camera(camera) => camera.title(),
            Self::HighlightNext => "Highlight the next item in the view",
            Self::HighlightPrevious => "Highlight the previous item in the view",
            Self::ActivateHighlighted => "Select the highlighted item",
            Self::HideSelection => "Hide selection",
            Self::ToggleVisibility => "Hide or show feature",
            Self::ShowAll => "Show everything",
            Self::TogglePrincipal => "Hide or show principal planes, axes and origin",
            Self::ToggleProjection => "Switch between perspective and orthographic",
            Self::OpenSample(sample) => return format!("Open the {} sample", sample.title()),
            Self::OpenRecent(slot) => return slot.title(),
            Self::RecoverUnsaved => "Recover Unsaved Work…",
            Self::CancelExport => "Cancel the export",
            Self::Recompute => "Recompute the model",
            Self::CancelRecompute => "Cancel the recompute",
            Self::RenameFeature => "Rename feature",
            Self::MoveFeatureUp => "Move feature up",
            Self::MoveFeatureDown => "Move feature down",
            Self::DeleteFeature => "Delete feature",
            Self::EditFeature => "Edit feature",
            Self::CloseFeature => "Finish editing feature",
            Self::DetachSketch => "Detach sketch",
            Self::PlaceSketch => "Place sketch on selected plane or face",
            Self::UseSelectedAxis => "Revolve about selected axis",
            Self::DatumUseSelected => "Base datum on selection",
            Self::DatumTurnAboutSelected => "Turn datum plane about selected axis",
            Self::AddParameter => "Add parameter",
            Self::DeleteParameter => "Delete parameter",
            Self::ShowFirstFailed => "Go to the first failed feature",
            Self::DismissNotice => "Dismiss the notice",
            Self::DismissTip => "Dismiss the tip",
            Self::HideTips => "Hide tips",
            Self::Welcome => "Welcome and samples…",
            Self::About => "About caditor",
        };
        fixed.to_owned()
    }

    pub fn category(self) -> Category {
        match self {
            Self::New
            | Self::Open
            | Self::Save
            | Self::SaveAs
            | Self::VersionHistory
            | Self::Import
            | Self::Export
            | Self::Preferences
            | Self::KeyboardShortcuts
            | Self::Quit
            | Self::OpenSample(_)
            | Self::OpenRecent(_)
            | Self::RecoverUnsaved
            | Self::CancelExport => Category::File,
            Self::Welcome | Self::About | Self::DismissTip | Self::HideTips => Category::Help,
            Self::Palette
            | Self::Undo
            | Self::Redo
            | Self::DeleteSelection
            | Self::DismissNotice => Category::Edit,
            Self::FitView
            | Self::LargerInterface
            | Self::SmallerInterface
            | Self::NormalInterface
            | Self::View(_)
            | Self::Camera(_)
            | Self::HighlightNext
            | Self::HighlightPrevious
            | Self::ActivateHighlighted
            | Self::HideSelection
            | Self::ToggleVisibility
            | Self::ShowAll
            | Self::TogglePrincipal
            | Self::ToggleProjection => Category::View,
            Self::NewSketch
            | Self::Extrude
            | Self::Revolve
            | Self::Fillet
            | Self::Chamfer
            | Self::Shell
            | Self::DatumPlane
            | Self::DatumAxis
            | Self::Recompute
            | Self::CancelRecompute
            | Self::RenameFeature
            | Self::MoveFeatureUp
            | Self::MoveFeatureDown
            | Self::DeleteFeature
            | Self::EditFeature
            | Self::CloseFeature
            | Self::DetachSketch
            | Self::PlaceSketch
            | Self::UseSelectedAxis
            | Self::DatumUseSelected
            | Self::DatumTurnAboutSelected
            | Self::AddParameter
            | Self::DeleteParameter
            | Self::ShowFirstFailed => Category::Model,
            Self::FinishSketch
            | Self::ReverseArc
            | Self::Construction
            | Self::MoveGeometry
            | Self::SelectAll
            | Self::SketchTool(_) => Category::Sketch,
            Self::Constraint(_) => Category::Constraint,
        }
    }

    pub fn scope(self) -> Scope {
        match self {
            Self::FinishSketch
            | Self::ReverseArc
            | Self::Construction
            | Self::MoveGeometry
            | Self::SelectAll
            | Self::SketchTool(_)
            | Self::Constraint(_) => Scope::Sketch,
            _ => Scope::Anywhere,
        }
    }

    pub fn default_shortcuts(self) -> Vec<KeyboardShortcut> {
        let command = |key| KeyboardShortcut::new(Modifiers::COMMAND, key);
        let command_shift = |key| KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, key);
        let plain = |key| KeyboardShortcut::new(Modifiers::NONE, key);
        match self {
            Self::Palette => vec![command_shift(Key::P)],
            Self::New => vec![command(Key::N)],
            Self::Open => vec![command(Key::O)],
            Self::Save => vec![command(Key::S)],
            Self::SaveAs => vec![command_shift(Key::S)],
            Self::Import => vec![command(Key::I)],
            Self::Export => vec![command(Key::E)],
            Self::Preferences => vec![command(Key::Comma)],
            Self::Quit => vec![command(Key::Q)],
            Self::Undo => vec![command(Key::Z)],
            Self::Redo => vec![command_shift(Key::Z), command(Key::Y)],
            Self::FitView => vec![plain(Key::F)],
            Self::ToggleProjection => vec![plain(Key::O)],
            Self::LargerInterface => vec![command(Key::Plus), command(Key::Equals)],
            Self::SmallerInterface => vec![command(Key::Minus)],
            Self::NormalInterface => vec![command(Key::Num0)],
            Self::View(view) => vec![KeyboardShortcut::new(Modifiers::ALT, view.key())],
            Self::Camera(camera) => vec![camera.shortcut()],
            Self::HighlightNext => vec![plain(Key::N)],
            Self::HighlightPrevious => vec![KeyboardShortcut::new(Modifiers::SHIFT, Key::N)],
            Self::ActivateHighlighted => vec![plain(Key::Space)],
            Self::HideSelection => vec![plain(Key::H)],
            Self::ShowAll => vec![KeyboardShortcut::new(Modifiers::ALT, Key::H)],
            Self::SketchTool(tool) => tool_key(tool).map(plain).into_iter().collect(),
            Self::Constraint(tool) => vec![KeyboardShortcut::new(
                Modifiers::SHIFT,
                constraint_key(tool),
            )],
            Self::DeleteSelection => vec![plain(Key::Delete), plain(Key::Backspace)],
            Self::ReverseArc => vec![plain(Key::X)],
            Self::Construction => vec![plain(Key::Q)],
            Self::MoveGeometry => vec![plain(Key::M)],
            Self::SelectAll => vec![command(Key::A)],
            Self::RenameFeature => vec![plain(Key::F2)],
            Self::Recompute => vec![plain(Key::F5)],
            Self::EditFeature => vec![plain(Key::E)],
            Self::ShowFirstFailed => vec![plain(Key::F8)],
            Self::VersionHistory
            | Self::KeyboardShortcuts
            | Self::NewSketch
            | Self::FinishSketch
            | Self::Extrude
            | Self::Revolve
            | Self::Fillet
            | Self::Chamfer
            | Self::Shell
            | Self::DatumPlane
            | Self::DatumAxis
            | Self::OpenSample(_)
            | Self::OpenRecent(_)
            | Self::RecoverUnsaved
            | Self::CancelExport
            | Self::CancelRecompute
            | Self::MoveFeatureUp
            | Self::MoveFeatureDown
            | Self::DeleteFeature
            | Self::ToggleVisibility
            | Self::TogglePrincipal
            | Self::CloseFeature
            | Self::DetachSketch
            | Self::PlaceSketch
            | Self::UseSelectedAxis
            | Self::DatumUseSelected
            | Self::DatumTurnAboutSelected
            | Self::AddParameter
            | Self::DeleteParameter
            | Self::DismissNotice
            | Self::DismissTip
            | Self::HideTips
            | Self::Welcome
            | Self::About => Vec::new(),
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        Self::all().find(|command| command.id() == id)
    }
}

fn tool_key(tool: Tool) -> Option<Key> {
    match tool {
        Tool::Select => None,
        Tool::Point => Some(Key::P),
        Tool::Line => Some(Key::L),
        Tool::Rectangle => Some(Key::R),
        Tool::Circle => Some(Key::C),
        Tool::Arc => Some(Key::A),
        Tool::Spline => Some(Key::S),
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
        command.scope() == Scope::Anywhere || self.editing_sketch
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

pub struct CommandFrame<'a> {
    keymap: &'a Keymap,
    triggered: Vec<Command>,
    offers: Vec<Offer>,
    refused: Vec<(Command, String)>,
}

impl<'a> CommandFrame<'a> {
    pub fn new(keymap: &'a Keymap, triggered: Vec<Command>) -> Self {
        Self {
            keymap,
            triggered,
            offers: Vec::new(),
            refused: Vec::new(),
        }
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
            Some(vec!["E".to_owned()])
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
            None
        );
        assert_eq!(
            found(
                press(Key::S, Modifiers::COMMAND | Modifiers::SHIFT),
                &sketch
            ),
            Some(Command::SaveAs)
        );
        assert_eq!(found(press(Key::S, Modifiers::ALT), &sketch), None);
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
