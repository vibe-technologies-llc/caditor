use std::sync::LazyLock;

use caditor_document::{Datum, FeatureKind, SolidFeature};
use egui::Id;

use crate::{analysis::Kind, commands::Command, editing::Tool, variants::all_variants};

const SNIPPET_BEFORE: usize = 40;
const SNIPPET_LENGTH: usize = 140;
const MOST_HITS: usize = 30;
const TITLE_WEIGHT: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Page {
    Start,
    Window,
    Navigation,
    Display,
    Selection,
    Keyboard,
    Undo,
    Sketches,
    Snapping,
    Constraints,
    Dimensions,
    SketchStatus,
    SketchEditing,
    AutomaticConstraints,
    Point,
    Line,
    Rectangle,
    Circle,
    Arcs,
    Slot,
    Polygon,
    Spline,
    Conic,
    Ellipses,
    TrimAndExtend,
    Offset,
    SketchMirror,
    SketchPatterns,
    TangentCircle,
    Gear,
    SketchFillet,
    ProjectAndIntersect,
    BlendCurve,
    Parameters,
    Expressions,
    Configurations,
    FeatureTree,
    Extrude,
    Revolve,
    Hole,
    Thread,
    FilletAndChamfer,
    Shell,
    OffsetFace,
    Primitives,
    Combine,
    MoveAndCopy,
    Mate,
    Mirror,
    Split,
    SplitFace,
    Scale,
    Patterns,
    Datums,
    CoordinateSystems,
    Bodies,
    ImportedBodies,
    Measure,
    Interference,
    FaceAnalysis,
    CurvatureComb,
    Isocurves,
    SectionView,
    Saving,
    VersionHistory,
    Importing,
    Exporting,
    Templates,
    WindowsDefender,
    CommandLine,
    Preferences,
    Accessibility,
}

all_variants!(
    Page: Start,
    Window,
    Navigation,
    Display,
    Selection,
    Keyboard,
    Undo,
    Sketches,
    Snapping,
    Constraints,
    Dimensions,
    SketchStatus,
    SketchEditing,
    AutomaticConstraints,
    Point,
    Line,
    Rectangle,
    Circle,
    Arcs,
    Slot,
    Polygon,
    Spline,
    Conic,
    Ellipses,
    TrimAndExtend,
    Offset,
    SketchMirror,
    SketchPatterns,
    TangentCircle,
    Gear,
    SketchFillet,
    ProjectAndIntersect,
    BlendCurve,
    Parameters,
    Expressions,
    Configurations,
    FeatureTree,
    Extrude,
    Revolve,
    Hole,
    Thread,
    FilletAndChamfer,
    Shell,
    OffsetFace,
    Primitives,
    Combine,
    MoveAndCopy,
    Mate,
    Mirror,
    Split,
    SplitFace,
    Scale,
    Patterns,
    Datums,
    CoordinateSystems,
    Bodies,
    ImportedBodies,
    Measure,
    Interference,
    FaceAnalysis,
    CurvatureComb,
    Isocurves,
    SectionView,
    Saving,
    VersionHistory,
    Importing,
    Exporting,
    Templates,
    WindowsDefender,
    CommandLine,
    Preferences,
    Accessibility,
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chapter {
    Basics,
    Sketching,
    SketchTools,
    Parameters,
    Modelling,
    Inspecting,
    Files,
    Settings,
}

all_variants!(
    Chapter: Basics,
    Sketching,
    SketchTools,
    Parameters,
    Modelling,
    Inspecting,
    Files,
    Settings,
);

impl Chapter {
    pub fn title(self) -> &'static str {
        match self {
            Self::Basics => "The basics",
            Self::Sketching => "Sketching",
            Self::SketchTools => "Sketch tools",
            Self::Parameters => "Parameters and expressions",
            Self::Modelling => "Modelling",
            Self::Inspecting => "Inspecting",
            Self::Files => "Files",
            Self::Settings => "Settings",
        }
    }

    pub fn pages(self) -> impl Iterator<Item = Page> {
        Page::ALL
            .into_iter()
            .filter(move |page| page.chapter() == self)
    }
}

impl Page {
    pub fn id(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Window => "window",
            Self::Navigation => "navigation",
            Self::Display => "display",
            Self::Selection => "selection",
            Self::Keyboard => "keyboard",
            Self::Undo => "undo",
            Self::Sketches => "sketches",
            Self::Snapping => "snapping",
            Self::Constraints => "constraints",
            Self::Dimensions => "dimensions",
            Self::SketchStatus => "sketch-status",
            Self::SketchEditing => "sketch-editing",
            Self::AutomaticConstraints => "automatic-constraints",
            Self::Point => "point",
            Self::Line => "line",
            Self::Rectangle => "rectangle",
            Self::Circle => "circle",
            Self::Arcs => "arcs",
            Self::Slot => "slot",
            Self::Polygon => "polygon",
            Self::Spline => "spline",
            Self::Conic => "conic",
            Self::Ellipses => "ellipses",
            Self::TrimAndExtend => "trim-and-extend",
            Self::Offset => "offset",
            Self::SketchMirror => "sketch-mirror",
            Self::SketchPatterns => "sketch-patterns",
            Self::TangentCircle => "tangent-circle",
            Self::Gear => "gear",
            Self::SketchFillet => "sketch-fillet",
            Self::ProjectAndIntersect => "project-and-intersect",
            Self::BlendCurve => "blend-curve",
            Self::Parameters => "parameters",
            Self::Expressions => "expressions",
            Self::Configurations => "configurations",
            Self::FeatureTree => "feature-tree",
            Self::Extrude => "extrude",
            Self::Revolve => "revolve",
            Self::Hole => "hole",
            Self::Thread => "thread",
            Self::FilletAndChamfer => "fillet-and-chamfer",
            Self::Shell => "shell",
            Self::OffsetFace => "offset-face",
            Self::Primitives => "primitives",
            Self::Combine => "combine",
            Self::MoveAndCopy => "move-and-copy",
            Self::Mate => "mate",
            Self::Mirror => "mirror",
            Self::Split => "split",
            Self::SplitFace => "split-face",
            Self::Scale => "scale",
            Self::Patterns => "patterns",
            Self::Datums => "datums",
            Self::CoordinateSystems => "coordinate-systems",
            Self::Bodies => "bodies",
            Self::ImportedBodies => "imported-bodies",
            Self::Measure => "measure",
            Self::Interference => "interference",
            Self::FaceAnalysis => "face-analysis",
            Self::CurvatureComb => "curvature-comb",
            Self::Isocurves => "isocurves",
            Self::SectionView => "section-view",
            Self::Saving => "saving",
            Self::VersionHistory => "version-history",
            Self::Importing => "importing",
            Self::Exporting => "exporting",
            Self::Templates => "templates",
            Self::WindowsDefender => "windows-defender",
            Self::CommandLine => "command-line",
            Self::Preferences => "preferences",
            Self::Accessibility => "accessibility",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|page| page.id() == id)
    }

    fn source(self) -> &'static str {
        match self {
            Self::Start => include_str!("../guide/start.md"),
            Self::Window => include_str!("../guide/window.md"),
            Self::Navigation => include_str!("../guide/navigation.md"),
            Self::Display => include_str!("../guide/display.md"),
            Self::Selection => include_str!("../guide/selection.md"),
            Self::Keyboard => include_str!("../guide/keyboard.md"),
            Self::Undo => include_str!("../guide/undo.md"),
            Self::Sketches => include_str!("../guide/sketches.md"),
            Self::Snapping => include_str!("../guide/snapping.md"),
            Self::Constraints => include_str!("../guide/constraints.md"),
            Self::Dimensions => include_str!("../guide/dimensions.md"),
            Self::SketchStatus => include_str!("../guide/sketch-status.md"),
            Self::SketchEditing => include_str!("../guide/sketch-editing.md"),
            Self::AutomaticConstraints => include_str!("../guide/automatic-constraints.md"),
            Self::Point => include_str!("../guide/point.md"),
            Self::Line => include_str!("../guide/line.md"),
            Self::Rectangle => include_str!("../guide/rectangle.md"),
            Self::Circle => include_str!("../guide/circle.md"),
            Self::Arcs => include_str!("../guide/arcs.md"),
            Self::Slot => include_str!("../guide/slot.md"),
            Self::Polygon => include_str!("../guide/polygon.md"),
            Self::Spline => include_str!("../guide/spline.md"),
            Self::Conic => include_str!("../guide/conic.md"),
            Self::Ellipses => include_str!("../guide/ellipses.md"),
            Self::TrimAndExtend => include_str!("../guide/trim-and-extend.md"),
            Self::Offset => include_str!("../guide/offset.md"),
            Self::SketchMirror => include_str!("../guide/sketch-mirror.md"),
            Self::SketchPatterns => include_str!("../guide/sketch-patterns.md"),
            Self::TangentCircle => include_str!("../guide/tangent-circle.md"),
            Self::Gear => include_str!("../guide/gear.md"),
            Self::SketchFillet => include_str!("../guide/sketch-fillet.md"),
            Self::ProjectAndIntersect => include_str!("../guide/project-and-intersect.md"),
            Self::BlendCurve => include_str!("../guide/blend-curve.md"),
            Self::Parameters => include_str!("../guide/parameters.md"),
            Self::Expressions => include_str!("../guide/expressions.md"),
            Self::Configurations => include_str!("../guide/configurations.md"),
            Self::FeatureTree => include_str!("../guide/feature-tree.md"),
            Self::Extrude => include_str!("../guide/extrude.md"),
            Self::Revolve => include_str!("../guide/revolve.md"),
            Self::Hole => include_str!("../guide/hole.md"),
            Self::Thread => include_str!("../guide/thread.md"),
            Self::FilletAndChamfer => include_str!("../guide/fillet-and-chamfer.md"),
            Self::Shell => include_str!("../guide/shell.md"),
            Self::OffsetFace => include_str!("../guide/offset-face.md"),
            Self::Primitives => include_str!("../guide/primitives.md"),
            Self::Combine => include_str!("../guide/combine.md"),
            Self::MoveAndCopy => include_str!("../guide/move-and-copy.md"),
            Self::Mate => include_str!("../guide/mate.md"),
            Self::Mirror => include_str!("../guide/mirror.md"),
            Self::Split => include_str!("../guide/split.md"),
            Self::SplitFace => include_str!("../guide/split-face.md"),
            Self::Scale => include_str!("../guide/scale.md"),
            Self::Patterns => include_str!("../guide/patterns.md"),
            Self::Datums => include_str!("../guide/datums.md"),
            Self::CoordinateSystems => include_str!("../guide/coordinate-systems.md"),
            Self::Bodies => include_str!("../guide/bodies.md"),
            Self::ImportedBodies => include_str!("../guide/imported-bodies.md"),
            Self::Measure => include_str!("../guide/measure.md"),
            Self::Interference => include_str!("../guide/interference.md"),
            Self::FaceAnalysis => include_str!("../guide/face-analysis.md"),
            Self::CurvatureComb => include_str!("../guide/curvature-comb.md"),
            Self::Isocurves => include_str!("../guide/isocurves.md"),
            Self::SectionView => include_str!("../guide/section-view.md"),
            Self::Saving => include_str!("../guide/saving.md"),
            Self::VersionHistory => include_str!("../guide/version-history.md"),
            Self::Importing => include_str!("../guide/importing.md"),
            Self::Exporting => include_str!("../guide/exporting.md"),
            Self::Templates => include_str!("../guide/templates.md"),
            Self::WindowsDefender => include_str!("../guide/windows-defender.md"),
            Self::CommandLine => include_str!("../guide/command-line.md"),
            Self::Preferences => include_str!("../guide/preferences.md"),
            Self::Accessibility => include_str!("../guide/accessibility.md"),
        }
    }

    pub fn chapter(self) -> Chapter {
        match self {
            Self::Start
            | Self::Window
            | Self::Navigation
            | Self::Display
            | Self::Selection
            | Self::Keyboard
            | Self::Undo => Chapter::Basics,
            Self::Sketches
            | Self::Snapping
            | Self::Constraints
            | Self::Dimensions
            | Self::SketchStatus
            | Self::SketchEditing
            | Self::AutomaticConstraints => Chapter::Sketching,
            Self::Point
            | Self::Line
            | Self::Rectangle
            | Self::Circle
            | Self::Arcs
            | Self::Slot
            | Self::Polygon
            | Self::Spline
            | Self::Conic
            | Self::Ellipses
            | Self::TrimAndExtend
            | Self::Offset
            | Self::SketchMirror
            | Self::SketchPatterns
            | Self::TangentCircle
            | Self::Gear
            | Self::SketchFillet
            | Self::ProjectAndIntersect
            | Self::BlendCurve => Chapter::SketchTools,
            Self::Parameters | Self::Expressions | Self::Configurations => Chapter::Parameters,
            Self::FeatureTree
            | Self::Extrude
            | Self::Revolve
            | Self::Hole
            | Self::Thread
            | Self::FilletAndChamfer
            | Self::Shell
            | Self::OffsetFace
            | Self::Primitives
            | Self::Combine
            | Self::MoveAndCopy
            | Self::Mate
            | Self::Mirror
            | Self::Split
            | Self::SplitFace
            | Self::Scale
            | Self::Patterns
            | Self::Datums
            | Self::CoordinateSystems
            | Self::Bodies
            | Self::ImportedBodies => Chapter::Modelling,
            Self::Measure
            | Self::Interference
            | Self::FaceAnalysis
            | Self::CurvatureComb
            | Self::Isocurves
            | Self::SectionView => Chapter::Inspecting,
            Self::Saving
            | Self::VersionHistory
            | Self::Importing
            | Self::Exporting
            | Self::Templates
            | Self::WindowsDefender
            | Self::CommandLine => Chapter::Files,
            Self::Preferences | Self::Accessibility => Chapter::Settings,
        }
    }

    pub fn of_tool(tool: Tool) -> Self {
        match tool {
            Tool::Select => Self::Sketches,
            Tool::Point => Self::Point,
            Tool::Line => Self::Line,
            Tool::Rectangle => Self::Rectangle,
            Tool::Circle => Self::Circle,
            Tool::Arc | Tool::ThreePointArc | Tool::TangentArc => Self::Arcs,
            Tool::Slot => Self::Slot,
            Tool::Polygon => Self::Polygon,
            Tool::Spline => Self::Spline,
            Tool::Conic => Self::Conic,
            Tool::Ellipse | Tool::EllipticalArc => Self::Ellipses,
            Tool::Trim | Tool::Extend => Self::TrimAndExtend,
            Tool::Offset => Self::Offset,
            Tool::Mirror => Self::SketchMirror,
            Tool::RectangularPattern | Tool::CircularPattern => Self::SketchPatterns,
            Tool::TangentCircle => Self::TangentCircle,
            Tool::Gear => Self::Gear,
            Tool::Fillet | Tool::Chamfer => Self::SketchFillet,
            Tool::Project | Tool::Intersect => Self::ProjectAndIntersect,
            Tool::Dimension => Self::Dimensions,
            Tool::BlendCurve => Self::BlendCurve,
        }
    }

    pub fn of_feature(kind: &FeatureKind) -> Self {
        match kind {
            FeatureKind::Sketch(_) => Self::Sketches,
            FeatureKind::Solid(SolidFeature::Extrude(_)) => Self::Extrude,
            FeatureKind::Solid(SolidFeature::Revolve(_)) => Self::Revolve,
            FeatureKind::Blend(_) => Self::FilletAndChamfer,
            FeatureKind::Shell(_) => Self::Shell,
            FeatureKind::OffsetFace(_) => Self::OffsetFace,
            FeatureKind::Primitive(_) => Self::Primitives,
            FeatureKind::Combine(_) => Self::Combine,
            FeatureKind::Move(_) => Self::MoveAndCopy,
            FeatureKind::Mate(_) => Self::Mate,
            FeatureKind::Mirror(_) => Self::Mirror,
            FeatureKind::Split(_) => Self::Split,
            FeatureKind::SplitFace(_) => Self::SplitFace,
            FeatureKind::Scale(_) => Self::Scale,
            FeatureKind::Hole(_) => Self::Hole,
            FeatureKind::Pattern(_) => Self::Patterns,
            FeatureKind::Datum(Datum::Frame(_)) => Self::CoordinateSystems,
            FeatureKind::Datum(
                Datum::Plane(_)
                | Datum::PlaneThrough(_)
                | Datum::Axis(_)
                | Datum::Point(_)
                | Datum::PointBy(_),
            ) => Self::Datums,
            FeatureKind::Import(_) => Self::ImportedBodies,
            FeatureKind::Remove(_) => Self::Bodies,
            FeatureKind::Thread(_) => Self::Thread,
            FeatureKind::Measurement(_) => Self::Measure,
        }
    }

    pub fn of_analysis(kind: Kind) -> Self {
        match kind {
            Kind::Draft
            | Kind::Radius
            | Kind::Reach
            | Kind::Curvature
            | Kind::Zebra
            | Kind::Chrome => Self::FaceAnalysis,
        }
    }

    pub fn of_panel(panel: SidePanel) -> Self {
        match panel {
            SidePanel::Measure => Self::Measure,
            SidePanel::Interference => Self::Interference,
            SidePanel::Analysis(kind) => Self::of_analysis(kind),
            SidePanel::Comb => Self::CurvatureComb,
            SidePanel::Isocurves => Self::Isocurves,
            SidePanel::Section => Self::SectionView,
            SidePanel::Tidying => Self::AutomaticConstraints,
        }
    }

    pub fn title(self) -> &'static str {
        parsed(self).map_or(self.id(), |parsed| parsed.title.as_str())
    }

    pub fn blocks(self) -> &'static [Block] {
        parsed(self).map_or(&[], |parsed| parsed.blocks.as_slice())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidePanel {
    Measure,
    Interference,
    Analysis(Kind),
    Comb,
    Isocurves,
    Section,
    Tidying,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Context {
    Tool(Tool),
    Feature(Page),
    Panel(SidePanel),
    Sketch,
    ChoosingPlane,
    TreeRow(Page),
}

impl Context {
    pub fn page(self) -> Page {
        match self {
            Self::Tool(tool) => Page::of_tool(tool),
            Self::Feature(page) | Self::TreeRow(page) => page,
            Self::Panel(panel) => Page::of_panel(panel),
            Self::Sketch | Self::ChoosingPlane => Page::Sketches,
        }
    }
}

pub struct Situation<'a> {
    pub tool: Option<Tool>,
    pub choosing_plane: bool,
    pub open_feature: Option<&'a FeatureKind>,
    pub panels: &'a [SidePanel],
    pub tree_row: Option<&'a FeatureKind>,
}

pub fn context(situation: &Situation<'_>) -> Option<Context> {
    let drawing_tool = situation.tool.filter(|tool| *tool != Tool::Select);
    drawing_tool
        .map(Context::Tool)
        .or_else(|| {
            situation
                .open_feature
                .map(|kind| Context::Feature(Page::of_feature(kind)))
        })
        .or_else(|| situation.panels.first().copied().map(Context::Panel))
        .or_else(|| situation.tool.map(|_| Context::Sketch))
        .or_else(|| situation.choosing_plane.then_some(Context::ChoosingPlane))
        .or_else(|| {
            situation
                .tree_row
                .map(|kind| Context::TreeRow(Page::of_feature(kind)))
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Span {
    Text(String),
    Strong(String),
    Code(String),
    Link { text: String, target: Target },
    Command(Named),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Page(Page),
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Named {
    Command(Command),
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Heading(String),
    Paragraph(Vec<Span>),
    Item(Vec<Span>),
}

#[derive(Debug)]
pub struct Parsed {
    pub title: String,
    pub blocks: Vec<Block>,
    searched: String,
}

static GUIDE: LazyLock<Vec<(Page, Parsed)>> = LazyLock::new(|| {
    Page::ALL
        .into_iter()
        .map(|page| (page, parse(page.source())))
        .collect()
});

fn parsed(page: Page) -> Option<&'static Parsed> {
    GUIDE
        .iter()
        .find(|(listed, _)| *listed == page)
        .map(|(_, parsed)| parsed)
}

enum Open {
    Paragraph(Vec<String>),
    Item(Vec<String>),
}

pub fn parse(source: &str) -> Parsed {
    let mut title = String::new();
    let mut blocks = Vec::new();
    let mut open: Option<Open> = None;
    let close = |open: &mut Option<Open>, blocks: &mut Vec<Block>| match open.take() {
        Some(Open::Paragraph(lines)) => blocks.push(Block::Paragraph(spans(&lines.join(" ")))),
        Some(Open::Item(lines)) => blocks.push(Block::Item(spans(&lines.join(" ")))),
        None => {}
    };
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            close(&mut open, &mut blocks);
        } else if let Some(heading) = line.strip_prefix("## ") {
            close(&mut open, &mut blocks);
            blocks.push(Block::Heading(heading.trim().to_owned()));
        } else if let Some(name) = line.strip_prefix("# ") {
            close(&mut open, &mut blocks);
            title = name.trim().to_owned();
        } else if let Some(item) = line.strip_prefix("- ") {
            close(&mut open, &mut blocks);
            open = Some(Open::Item(vec![item.trim().to_owned()]));
        } else {
            match &mut open {
                Some(Open::Paragraph(lines) | Open::Item(lines)) => {
                    lines.push(trimmed.to_owned());
                }
                None => open = Some(Open::Paragraph(vec![trimmed.to_owned()])),
            }
        }
    }
    close(&mut open, &mut blocks);
    let searched = std::iter::once(title.clone())
        .chain(blocks.iter().map(block_text))
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();
    Parsed {
        title,
        blocks,
        searched,
    }
}

pub fn spans(text: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut plain = String::new();
    let mut rest = text;
    while let Some(next) = rest.chars().next() {
        if let Some((span, after)) = marked(rest) {
            if !plain.is_empty() {
                spans.push(Span::Text(std::mem::take(&mut plain)));
            }
            spans.push(span);
            rest = after;
        } else {
            plain.push(next);
            rest = rest.get(next.len_utf8()..).unwrap_or_default();
        }
    }
    if !plain.is_empty() {
        spans.push(Span::Text(plain));
    }
    spans
}

fn marked(text: &str) -> Option<(Span, &str)> {
    if let Some(inner) = text.strip_prefix("**") {
        let (strong, after) = inner.split_once("**")?;
        return Some((Span::Strong(strong.to_owned()), after));
    }
    if let Some(inner) = text.strip_prefix('`') {
        let (code, after) = inner.split_once('`')?;
        return Some((Span::Code(code.to_owned()), after));
    }
    if let Some(inner) = text.strip_prefix("{command:") {
        let (id, after) = inner.split_once('}')?;
        let named =
            Command::from_id(id).map_or_else(|| Named::Unknown(id.to_owned()), Named::Command);
        return Some((Span::Command(named), after));
    }
    let inner = text.strip_prefix('[')?;
    let (shown, rest) = inner.split_once("](")?;
    let (id, after) = rest.split_once(')')?;
    let target = Page::from_id(id).map_or_else(|| Target::Unknown(id.to_owned()), Target::Page);
    Some((
        Span::Link {
            text: shown.to_owned(),
            target,
        },
        after,
    ))
}

pub fn span_text(span: &Span) -> String {
    match span {
        Span::Text(text) | Span::Strong(text) | Span::Code(text) => text.clone(),
        Span::Link { text, .. } => text.clone(),
        Span::Command(Named::Command(command)) => command.title(),
        Span::Command(Named::Unknown(id)) => id.clone(),
    }
}

pub fn block_text(block: &Block) -> String {
    match block {
        Block::Heading(text) => text.clone(),
        Block::Paragraph(spans) | Block::Item(spans) => spans.iter().map(span_text).collect(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub page: Page,
    pub snippet: String,
}

pub fn search(query: &str) -> Vec<Hit> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<(usize, Hit)> = GUIDE
        .iter()
        .filter(|(_, parsed)| words.iter().all(|word| parsed.searched.contains(word)))
        .map(|(page, parsed)| {
            let title = parsed.title.to_lowercase();
            let score = words
                .iter()
                .map(|word| {
                    TITLE_WEIGHT * usize::from(title.contains(word.as_str()))
                        + parsed.searched.matches(word.as_str()).count()
                })
                .sum();
            let snippet = words
                .first()
                .map(|word| snippet(parsed, word))
                .unwrap_or_default();
            (
                score,
                Hit {
                    page: *page,
                    snippet,
                },
            )
        })
        .collect();
    scored.sort_by(|(left, _), (right, _)| right.cmp(left));
    scored
        .into_iter()
        .take(MOST_HITS)
        .map(|(_, hit)| hit)
        .collect()
}

fn snippet(parsed: &Parsed, word: &str) -> String {
    let text = parsed
        .blocks
        .iter()
        .map(block_text)
        .find(|text| text.to_lowercase().contains(word))
        .unwrap_or_default();
    let lower = text.to_lowercase();
    let found = lower.find(word).unwrap_or_default();
    let characters_before = lower
        .get(..found)
        .map_or(0, |before| before.chars().count());
    let start = characters_before.saturating_sub(SNIPPET_BEFORE);
    let shown: String = text.chars().skip(start).take(SNIPPET_LENGTH).collect();
    let leading = if start > 0 { "…" } else { "" };
    let trailing = if text.chars().count() > start + SNIPPET_LENGTH {
        "…"
    } else {
        ""
    };
    format!("{leading}{}{trailing}", shown.trim())
}

fn asked_key() -> Id {
    Id::new("guide-asked-page")
}

pub fn ask(ctx: &egui::Context, page: Page) {
    ctx.data_mut(|data| data.insert_temp(asked_key(), Some(page)));
    ctx.request_repaint();
}

pub fn take_asked(ctx: &egui::Context) -> Option<Page> {
    ctx.data_mut(|data| data.remove_temp::<Option<Page>>(asked_key()))
        .flatten()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::analysis::Kind;

    fn every_span() -> impl Iterator<Item = (Page, Span)> {
        Page::ALL.into_iter().flat_map(|page| {
            page.blocks()
                .iter()
                .flat_map(|block| match block {
                    Block::Heading(_) => Vec::new(),
                    Block::Paragraph(spans) | Block::Item(spans) => spans.clone(),
                })
                .map(move |span| (page, span))
        })
    }

    #[test]
    fn every_page_has_a_title_and_content_and_a_unique_id() {
        let ids: BTreeSet<&str> = Page::ALL.into_iter().map(Page::id).collect();
        assert_eq!(ids.len(), Page::ALL.len());
        for page in Page::ALL {
            assert!(!page.title().is_empty(), "{} has no title", page.id());
            assert_ne!(page.title(), page.id(), "{} has no title", page.id());
            assert!(page.blocks().len() >= 2, "{} says too little", page.id());
            assert_eq!(Page::from_id(page.id()), Some(page));
        }
    }

    #[test]
    fn every_link_names_a_page_and_every_command_exists() {
        let broken: Vec<String> = every_span()
            .filter_map(|(page, span)| match span {
                Span::Link {
                    target: Target::Unknown(id),
                    ..
                } => Some(format!("{} links to {id}", page.id())),
                Span::Command(Named::Unknown(id)) => {
                    Some(format!("{} names the command {id}", page.id()))
                }
                Span::Text(text)
                    if ["**", "`", "](", "{command"]
                        .iter()
                        .any(|mark| text.contains(mark)) =>
                {
                    Some(format!("{} has an unclosed mark in “{text}”", page.id()))
                }
                _ => None,
            })
            .collect();
        assert!(broken.is_empty(), "{broken:#?}");
    }

    #[test]
    fn every_page_is_reached_from_another_or_from_the_start() {
        let linked: BTreeSet<&str> = every_span()
            .filter_map(|(_, span)| match span {
                Span::Link {
                    target: Target::Page(page),
                    ..
                } => Some(page.id()),
                _ => None,
            })
            .collect();
        let unlinked: Vec<&str> = Page::ALL
            .into_iter()
            .filter(|page| *page != Page::Start && !linked.contains(page.id()))
            .map(Page::id)
            .collect();
        assert!(unlinked.is_empty(), "no page links to {unlinked:?}");
    }

    #[test]
    fn every_tool_panel_and_side_panel_has_a_page_of_its_own_chapter() {
        for tool in Tool::ALL {
            let page = Page::of_tool(tool);
            assert!(
                matches!(page.chapter(), Chapter::SketchTools | Chapter::Sketching),
                "{tool:?} goes to {}",
                page.id()
            );
            assert!(!page.blocks().is_empty());
        }
        for kind in Kind::ALL {
            assert_eq!(Page::of_analysis(kind).chapter(), Chapter::Inspecting);
        }
        for panel in [
            SidePanel::Measure,
            SidePanel::Interference,
            SidePanel::Comb,
            SidePanel::Isocurves,
            SidePanel::Section,
        ] {
            assert_eq!(Page::of_panel(panel).chapter(), Chapter::Inspecting);
        }
        assert_eq!(
            Page::of_panel(SidePanel::Tidying),
            Page::AutomaticConstraints
        );
        let listed: usize = Chapter::ALL
            .into_iter()
            .map(|chapter| chapter.pages().count())
            .sum();
        assert_eq!(listed, Page::ALL.len());
    }

    #[test]
    fn the_markup_reads_marks_links_commands_and_lists() {
        let parsed = parse(
            "# Title\n\nA **bold** `code` [link](start) and {command:file.save}.\n\
             second line\n\n## Heading\n\n- one\n  two\n- [x](nowhere)\n",
        );
        assert_eq!(parsed.title, "Title");
        assert_eq!(
            parsed.blocks,
            vec![
                Block::Paragraph(vec![
                    Span::Text("A ".to_owned()),
                    Span::Strong("bold".to_owned()),
                    Span::Text(" ".to_owned()),
                    Span::Code("code".to_owned()),
                    Span::Text(" ".to_owned()),
                    Span::Link {
                        text: "link".to_owned(),
                        target: Target::Page(Page::Start),
                    },
                    Span::Text(" and ".to_owned()),
                    Span::Command(Named::Command(Command::Save)),
                    Span::Text(". second line".to_owned()),
                ]),
                Block::Heading("Heading".to_owned()),
                Block::Item(vec![Span::Text("one two".to_owned())]),
                Block::Item(vec![Span::Link {
                    text: "x".to_owned(),
                    target: Target::Unknown("nowhere".to_owned()),
                }]),
            ]
        );
        assert_eq!(
            spans("an ** open"),
            vec![Span::Text("an ** open".to_owned())]
        );
    }

    #[test]
    fn search_finds_pages_by_title_first_and_quotes_where_the_word_is() {
        let hits = search("expression functions");
        assert_eq!(hits.first().map(|hit| hit.page), Some(Page::Expressions));
        assert!(search("   ").is_empty());
        assert!(search("zzzz-not-a-word").is_empty());
        let fillet = search("fillet");
        assert!(fillet.iter().any(|hit| hit.page == Page::FilletAndChamfer));
        assert!(
            fillet
                .iter()
                .all(|hit| hit.snippet.to_lowercase().contains("fillet")),
            "{fillet:#?}"
        );
    }

    #[test]
    fn the_context_prefers_the_tool_then_the_open_feature_then_side_panels() {
        let panels = [SidePanel::Measure];
        let mut situation = Situation {
            tool: Some(Tool::Line),
            choosing_plane: false,
            open_feature: None,
            panels: &panels,
            tree_row: None,
        };
        assert_eq!(context(&situation).map(Context::page), Some(Page::Line));
        situation.tool = Some(Tool::Select);
        assert_eq!(context(&situation).map(Context::page), Some(Page::Measure));
        situation.panels = &[];
        assert_eq!(context(&situation).map(Context::page), Some(Page::Sketches));
        situation.tool = None;
        assert_eq!(context(&situation), None);
        situation.choosing_plane = true;
        assert_eq!(context(&situation).map(Context::page), Some(Page::Sketches));
    }
}
