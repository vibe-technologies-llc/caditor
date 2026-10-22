use std::{borrow::Cow, collections::BTreeSet};

use caditor_document::{
    Document, Edit, FeatureId, FeatureKind, ImageSpot, ParameterOwner, Transaction, UndoMark,
};
use caditor_expression::ParameterId;
use caditor_sketch::Sketch;

use crate::{
    combine_tools,
    commands::{Command, Reason},
    model::{Action, Model, Notice, NoticeKind},
    queued::{Queued, Rerun},
    reference_picking::{Picking, Slot},
    selection::{Pickable, PrincipalPlane},
    shape_modes::{ShapeMode, ShapeModes},
    sketch_placement::{self, DatumTarget, FaceChoice},
    variants::all_variants,
    visibility::PrincipalView,
};

const NEW_SKETCH_PREFIX: &str = "Sketch";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tool {
    #[default]
    Select,
    Point,
    Line,
    Rectangle,
    Circle,
    Arc,
    ThreePointArc,
    TangentArc,
    Slot,
    Polygon,
    Spline,
    Ellipse,
    EllipticalArc,
    Conic,
    Trim,
    Extend,
    Offset,
    Mirror,
    RectangularPattern,
    CircularPattern,
    TangentCircle,
    Gear,
    Text,
    Fillet,
    Chamfer,
    Project,
    Intersect,
    Dimension,
    BlendCurve,
}

all_variants!(Tool: Select, Point, Line, Rectangle, Circle, Arc, ThreePointArc, TangentArc, Slot, Polygon, Spline, Ellipse, EllipticalArc, Conic, Trim, Extend, Offset, Mirror, RectangularPattern, CircularPattern, TangentCircle, Gear, Text, Fillet, Chamfer, Project, Intersect, Dimension, BlendCurve);

impl Tool {
    pub fn label(self) -> &'static str {
        match self {
            Self::Select => "Select",
            Self::Point => "Point",
            Self::Line => "Line",
            Self::Rectangle => "Rectangle",
            Self::Circle => "Circle",
            Self::Arc => "Arc",
            Self::ThreePointArc => "3-point arc",
            Self::TangentArc => "Tangent arc",
            Self::Ellipse => "Ellipse",
            Self::EllipticalArc => "Elliptical arc",
            Self::Conic => "Conic",
            Self::Slot => "Slot",
            Self::Polygon => "Polygon",
            Self::Spline => "Spline",
            Self::Trim => "Trim",
            Self::Extend => "Extend",
            Self::Offset => "Offset",
            Self::Mirror => "Mirror",
            Self::RectangularPattern => "Rectangular pattern",
            Self::CircularPattern => "Circular pattern",
            Self::TangentCircle => "Tangent circle",
            Self::Gear => "Spur gear",
            Self::Text => "Text",
            Self::Fillet => "Sketch fillet",
            Self::Chamfer => "Sketch chamfer",
            Self::Project => "Project",
            Self::Intersect => "Intersect",
            Self::Dimension => "Smart dimension",
            Self::BlendCurve => "Blend curve",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Select => {
                "Click geometry to select it, drag it to move it, or drag across empty space to \
                 select what the box takes in"
            }
            Self::Point => "Place points",
            Self::Line => "Draw connected lines, one click per corner",
            Self::Rectangle => {
                "Draw a rectangle from two opposite corners, from its centre and a corner, or \
                 from three points"
            }
            Self::Circle => {
                "Draw a circle from its centre and a point on it, or through two or three points"
            }
            Self::Arc => "Draw an arc from its centre, start and end",
            Self::ThreePointArc => "Draw an arc from its start and end through a third point",
            Self::TangentArc => {
                "Draw arcs that continue smoothly from the end of a line, arc or spline"
            }
            Self::Ellipse => {
                "Draw an ellipse from its centre, the end of its major axis and its minor radius"
            }
            Self::EllipticalArc => {
                "Draw part of an ellipse from its centre, the end of its major axis, its start \
                 and its end"
            }
            Self::Conic => {
                "Draw a conic from its two ends and its apex, where the tangents at its ends \
                 meet, shaped by a rho value"
            }
            Self::Slot => {
                "Draw a slot from the centres of its round ends, from its centre, or along an \
                 arc, then set its width"
            }
            Self::Polygon => {
                "Draw a regular polygon from its centre and a corner or a side's middle, or from \
                 one side"
            }
            Self::Spline => {
                "Draw a smooth curve shaped by control points, or passing through each point you \
                 place, open or closed"
            }
            Self::Trim => {
                "Click a piece of a line, circle, arc, ellipse or elliptical arc to cut it away up \
                 to the curves crossing it, splines included, or drag across several pieces"
            }
            Self::Extend => {
                "Click near the end of a line, arc or elliptical arc to lengthen it to the next \
                 curve in its way"
            }
            Self::Offset => {
                "Copy the selected chain of lines, arcs or circles at a distance to one or both \
                 sides, kept at that distance as the original changes; an ellipse's copy is a \
                 spline that does not follow it"
            }
            Self::Mirror => {
                "Copy the selected geometry mirrored about a line or axis, kept mirrored as the \
                 original changes"
            }
            Self::RectangularPattern => {
                "Repeat the selected geometry along one or two directions at a spacing, kept \
                 repeated as the original changes"
            }
            Self::CircularPattern => {
                "Repeat the selected geometry about a point, turned by equal steps, kept \
                 repeated as the original changes"
            }
            Self::TangentCircle => {
                "Draw a circle tangent to three lines, circles or arcs, or to two of them at a \
                 typed radius, kept tangent as they change"
            }
            Self::Gear => {
                "Draw an involute spur gear from its module, teeth and pressure angle, with its \
                 pitch, base, root and tip circles, ready to extrude; it stays editable and \
                 follows its parameters"
            }
            Self::Text => {
                "Write text in Inter, one or several lines, at a point, with a height, an \
                 angle, alignment, bold and italic; its letters are closed regions ready to extrude, and it stays editable"
            }
            Self::Fillet => {
                "Round the corner where two lines, arcs or elliptical arcs meet, or where two of \
                 them or circles cross or would meet, with an arc tangent to both"
            }
            Self::Chamfer => {
                "Cut the corner where two lines, arcs or elliptical arcs meet, or where two of \
                 them or circles cross or would meet, with a line at the distances you set"
            }
            Self::Project => {
                "Click an edge, corner or face of a body, or a curve of another sketch, to bring \
                 it into this sketch; it follows the original as the model changes"
            }
            Self::Intersect => {
                "Click a face to draw where the sketch plane cuts it, Shift-click for the whole \
                 body, or click a datum plane for the line it crosses the sketch in; the outline \
                 follows the model as it changes"
            }
            Self::Dimension => {
                "Click the geometry to dimension: a line for its length, a circle, arc or ellipse \
                 for its size, or two items for the distance or angle between them"
            }
            Self::BlendCurve => {
                "Join the ends of two lines, arcs or splines with a spline tangent to both, or \
                 also matching their curvature, kept smooth as they change"
            }
        }
    }

    pub fn draws(self) -> bool {
        match self {
            Self::Select
            | Self::Trim
            | Self::Extend
            | Self::Offset
            | Self::Mirror
            | Self::RectangularPattern
            | Self::CircularPattern
            | Self::TangentCircle
            | Self::Gear
            | Self::Text
            | Self::Fillet
            | Self::Chamfer
            | Self::Project
            | Self::Intersect
            | Self::Dimension
            | Self::BlendCurve => false,
            Self::Point
            | Self::Line
            | Self::Rectangle
            | Self::Circle
            | Self::Arc
            | Self::ThreePointArc
            | Self::TangentArc
            | Self::Ellipse
            | Self::EllipticalArc
            | Self::Conic
            | Self::Slot
            | Self::Polygon
            | Self::Spline => true,
        }
    }

    pub fn modifies(self) -> bool {
        self.trims() || self.reshapes()
    }

    pub fn trims(self) -> bool {
        matches!(self, Self::Trim | Self::Extend)
    }

    pub fn reshapes(self) -> bool {
        matches!(
            self,
            Self::Offset
                | Self::Mirror
                | Self::RectangularPattern
                | Self::CircularPattern
                | Self::TangentCircle
                | Self::Gear
                | Self::Text
                | Self::Fillet
                | Self::Chamfer
                | Self::BlendCurve
        )
    }

    pub fn projects(self) -> bool {
        matches!(self, Self::Project | Self::Intersect)
    }

    pub fn intersects(self) -> bool {
        self == Self::Intersect
    }

    pub fn dimensions(self) -> bool {
        self == Self::Dimension
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveSketch {
    pub feature: FeatureId,
    pub tool: Tool,
    pub construction: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EditingCommand {
    NewSketch(Option<PrincipalPlane>),
    NewSketchOnFace(FaceChoice),
    NewSketchOnDatum(DatumTarget),
    CancelNewSketch,
    ChooseCombined(Option<FeatureId>),
    CombineWith(FeatureId),
    CancelCombine,
    Enter(FeatureId),
    Finish,
    CancelSketch,
    SwitchBack(SwitchBack),
    SetTool(Tool),
    SetMode(ShapeMode),
    DrawConstruction(bool),
    OpenSolid(FeatureId),
    CloseSolid,
    CancelSolid,
    Pick(Picking),
    HoldPicked(Pickable),
    StopPicking,
    CalibrateFrom(ImageSpot),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Context {
    pub sketch: Option<FeatureId>,
    pub solid: Option<FeatureId>,
    pub choosing_plane: bool,
    pub choosing_bodies: bool,
    pub projecting: bool,
    pub intersecting: bool,
    pub selecting: bool,
    pub choosing_in_view: bool,
    pub choosing_profiles: bool,
    pub bodies_only: bool,
    pub principal: PrincipalView,
}

impl Context {
    pub fn with_principal(self, principal: PrincipalView) -> Self {
        Self { principal, ..self }
    }

    pub fn picks_shown_regions(self) -> bool {
        self.sketch.is_none()
            && !self.choosing_plane
            && !self.choosing_bodies
            && (self.choosing_profiles || (self.solid.is_none() && !self.choosing_in_view))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Combining {
    pub target: Option<FeatureId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Opened {
    pub feature: FeatureId,
    pub added: bool,
    mark: UndoMark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entered {
    pub feature: FeatureId,
    pub added: bool,
    mark: UndoMark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Closed {
    Sketch(Entered),
    Feature(Opened),
}

impl Closed {
    fn feature(self) -> FeatureId {
        match self {
            Self::Sketch(entered) => entered.feature,
            Self::Feature(opened) => opened.feature,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwitchBack {
    closed: Closed,
    since: UndoMark,
    session: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Switched {
    closed: Closed,
    since: UndoMark,
}

enum Discard {
    Nothing,
    Undo,
    Apply(Transaction),
}

const NOTHING_OPEN: &str = "no feature is open; open one with Edit feature first";
const NO_SKETCH: &str = "no sketch is being edited";
const HISTORY_GONE: &str =
    "the undo history no longer goes back to before the switch; use Undo instead";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CancelRefusal {
    #[error("{NOTHING_OPEN}")]
    NothingOpen,
    #[error("{NO_SKETCH}")]
    NoSketch,
    #[error(
        "the undo history no longer goes back to when {feature} was opened; undo its changes \
         one at a time instead"
    )]
    HistoryChanged { feature: String },
    #[error(
        "Undo went back past the point where {feature} was opened; redo back to it to cancel, or \
         undo its changes one at a time instead"
    )]
    UndoneBeyond { feature: String },
    #[error(
        "\"{step}\", made while {feature} was open, changed more than {feature}; undo the \
         changes one at a time instead"
    )]
    ChangedElsewhere { feature: String, step: String },
}

impl Reason for CancelRefusal {
    fn reason(&self) -> Cow<'static, str> {
        match self {
            Self::NothingOpen => Cow::Borrowed(NOTHING_OPEN),
            Self::NoSketch => Cow::Borrowed(NO_SKETCH),
            Self::HistoryChanged { .. }
            | Self::UndoneBeyond { .. }
            | Self::ChangedElsewhere { .. } => Cow::Owned(self.to_string()),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SketchEditing {
    active: Option<ActiveSketch>,
    entered: Option<Entered>,
    solid: Option<Opened>,
    picking: Option<Picking>,
    calibration_from: Option<ImageSpot>,
    choosing_plane: bool,
    combining: Option<Combining>,
    modes: ShapeModes,
    session: u64,
    switched: Option<Switched>,
}

impl SketchEditing {
    pub fn active(&self) -> Option<ActiveSketch> {
        self.active
    }

    pub fn feature(&self) -> Option<FeatureId> {
        self.active.map(|active| active.feature)
    }

    pub fn solid(&self) -> Option<FeatureId> {
        self.solid.map(|opened| opened.feature)
    }

    pub fn opened(&self) -> Option<Opened> {
        self.solid
    }

    pub fn cancelling(&self, model: &Model) -> Result<usize, CancelRefusal> {
        let opened = self.solid.ok_or(CancelRefusal::NothingOpen)?;
        changes_since(model, opened.feature, opened.added, opened.mark)
            .map_err(|refusal| refusal.unwrap_or(CancelRefusal::NothingOpen))
            .map(|steps| steps.len())
    }

    pub fn sketch_cancelling(&self, model: &Model) -> Result<usize, CancelRefusal> {
        self.sketch_changes(model).map(|steps| steps.len())
    }

    fn entered(&self) -> Option<Entered> {
        self.entered
            .filter(|entered| Some(entered.feature) == self.feature())
    }

    fn sketch_changes<'a>(&self, model: &'a Model) -> Result<Vec<&'a Transaction>, CancelRefusal> {
        let entered = self.entered().ok_or(CancelRefusal::NoSketch)?;
        changes_since(model, entered.feature, entered.added, entered.mark)
            .map_err(|refusal| refusal.unwrap_or(CancelRefusal::NoSketch))
    }

    pub fn modes(&self) -> ShapeModes {
        self.modes
    }

    pub fn context(&self) -> Context {
        Context {
            sketch: self.feature(),
            solid: self.solid(),
            choosing_plane: self.choosing_plane,
            choosing_bodies: self.combining.is_some(),
            projecting: self.active.is_some_and(|active| active.tool.projects()),
            intersecting: self.active.is_some_and(|active| active.tool.intersects()),
            selecting: self
                .active
                .is_some_and(|active| active.tool == Tool::Select),
            choosing_in_view: self.picking.is_some(),
            choosing_profiles: self
                .picking
                .is_some_and(|picking| picking.slot == Slot::LoftProfiles),
            bodies_only: false,
            principal: PrincipalView::default(),
        }
    }

    pub fn is_choosing_plane(&self) -> bool {
        self.choosing_plane
    }

    pub fn combining(&self) -> Option<Combining> {
        self.combining
    }

    pub fn picking(&self) -> Option<Picking> {
        self.picking
    }

    pub fn calibration_from(&self) -> Option<ImageSpot> {
        self.calibration_from.filter(|_| {
            self.picking
                .is_some_and(|picking| picking.slot == Slot::ImageCalibration)
        })
    }

    #[cfg(test)]
    pub fn editing(feature: FeatureId) -> Self {
        Self {
            active: Some(ActiveSketch {
                feature,
                tool: Tool::Select,
                construction: false,
            }),
            ..Self::default()
        }
    }

    pub fn perform(&mut self, command: EditingCommand, model: &mut Model) {
        let since = model.undo_mark();
        self.perform_after(command, model, since);
    }

    pub fn perform_after(&mut self, command: EditingCommand, model: &mut Model, since: UndoMark) {
        self.sync(model);
        let before = (
            self.feature().map(|feature| {
                self.entered().unwrap_or(Entered {
                    feature,
                    added: false,
                    mark: since,
                })
            }),
            self.solid,
        );
        match command {
            EditingCommand::NewSketch(Some(plane)) => self.create(plane, model, since),
            EditingCommand::NewSketchOnFace(face) => self.create_on_face(face, model, since),
            EditingCommand::NewSketchOnDatum(datum) => self.create_on_datum(datum, model, since),
            EditingCommand::NewSketch(None) => {
                self.leave_sketch();
                self.solid = None;
                self.combining = None;
                self.choosing_plane = true;
            }
            EditingCommand::CancelNewSketch => self.choosing_plane = false,
            EditingCommand::ChooseCombined(target) => {
                self.leave_sketch();
                self.solid = None;
                self.picking = None;
                self.choosing_plane = false;
                self.combining = Some(Combining { target });
            }
            EditingCommand::CombineWith(body) => self.combine_with(body, model, since),
            EditingCommand::CancelCombine => self.combining = None,
            EditingCommand::Enter(feature) => self.enter(feature, model, since),
            EditingCommand::Finish => self.finish(model),
            EditingCommand::CancelSketch => self.cancel_sketch(model),
            EditingCommand::SwitchBack(back) => self.switch_back(back, model),
            EditingCommand::SetTool(tool) => {
                if let Some(active) = &mut self.active {
                    active.tool = tool;
                }
            }
            EditingCommand::SetMode(mode) => {
                self.modes.set(mode);
                if let Some(active) = &mut self.active {
                    active.tool = mode.tool();
                }
            }
            EditingCommand::DrawConstruction(construction) => {
                if let Some(active) = &mut self.active {
                    active.construction = construction;
                }
            }
            EditingCommand::OpenSolid(feature) => self.open_solid(feature, model, since),
            EditingCommand::CloseSolid => self.solid = None,
            EditingCommand::CancelSolid => self.cancel(model),
            EditingCommand::Pick(picking) => {
                self.calibration_from = None;
                self.open_solid(picking.feature, model, since);
                if self.solid() == Some(picking.feature) {
                    self.picking = Some(picking);
                }
            }
            EditingCommand::HoldPicked(pickable) => {
                if let Some(picking) = &mut self.picking {
                    picking.hold(pickable);
                }
            }
            EditingCommand::StopPicking => {
                self.picking = None;
                self.calibration_from = None;
            }
            EditingCommand::CalibrateFrom(spot) => {
                self.calibration_from = Some(spot);
                if let Some(picking) = &mut self.picking {
                    picking.hold(Pickable::Datum(picking.feature));
                }
            }
        }
        self.drop_stale_picking();
        if switches(command) {
            self.note_switch(before, since);
        }
    }

    fn leave_sketch(&mut self) {
        self.active = None;
        self.entered = None;
    }

    fn note_switch(&mut self, before: (Option<Entered>, Option<Opened>), since: UndoMark) {
        if self.switched.is_some() {
            return;
        }
        let closed = match before {
            (Some(sketch), _) if self.feature() != Some(sketch.feature) => Closed::Sketch(sketch),
            (None, Some(opened)) if self.solid().is_none() => Closed::Feature(opened),
            _ => return,
        };
        self.switched = Some(Switched { closed, since });
    }

    pub fn announce_switch(&mut self, model: &mut Model, notice_before: Option<&Notice>) {
        let Some(switched) = self.switched.take() else {
            return;
        };
        let closed = switched.closed.feature();
        if self.feature() == Some(closed) || self.solid() == Some(closed) {
            return;
        }
        let document = model.document();
        let name = |feature| {
            document
                .feature(feature)
                .map(|feature| feature.name.clone())
        };
        let Some(closed_name) = name(closed) else {
            return;
        };
        let opened = self.feature().or(self.solid());
        if opened.is_some_and(|opened| document.dependents_of(&[closed]).contains(&opened)) {
            return;
        }
        let next = match opened {
            Some(feature) => match name(feature) {
                Some(next) => format!("edit {next}"),
                None => return,
            },
            None if self.choosing_plane => "start a new sketch".to_owned(),
            None => return,
        };
        let posted = model.notice();
        let warned = posted != notice_before
            && posted.is_some_and(|notice| {
                matches!(notice.kind, NoticeKind::Warning | NoticeKind::Error)
            });
        if warned {
            return;
        }
        let undo = model
            .undo_steps_since(switched.since)
            .filter(|steps| !steps.is_empty())
            .map(|_| SwitchBack {
                closed: switched.closed,
                since: switched.since,
                session: model.session(),
            });
        model.perform(Action::Inform(Notice {
            undo,
            ..Notice::info(format!("Finished {closed_name} to {next}."))
        }));
    }

    fn switch_back(&mut self, back: SwitchBack, model: &mut Model) {
        if back.session != model.session() {
            return;
        }
        let Some(steps) = model.undo_steps_since(back.since).map(|steps| steps.len()) else {
            model.perform(Action::Inform(Notice::warning(format!(
                "Undo: {HISTORY_GONE}."
            ))));
            return;
        };
        for _ in 0..steps {
            model.perform(Action::Undo);
        }
        self.sync(model);
        let document = model.document();
        match back.closed {
            Closed::Sketch(entered)
                if edited_sketch(document, entered.feature).is_some()
                    && document.is_active(entered.feature) =>
            {
                self.solid = None;
                self.picking = None;
                self.choosing_plane = false;
                self.combining = None;
                self.active = Some(ActiveSketch {
                    feature: entered.feature,
                    tool: Tool::Select,
                    construction: false,
                });
                self.entered = Some(entered);
            }
            Closed::Feature(opened)
                if opened_solid(document, opened.feature) && document.is_active(opened.feature) =>
            {
                self.leave_sketch();
                self.choosing_plane = false;
                self.combining = None;
                self.solid = Some(opened);
            }
            Closed::Sketch(_) | Closed::Feature(_) => {}
        }
        model.perform(Action::DismissNotice);
    }

    fn finish(&mut self, model: &mut Model) {
        if let Some(entered) = self.entered()
            && entered.added
            && edited_sketch(model.document(), entered.feature).is_some_and(is_empty)
        {
            self.remove_empty(entered, model);
        }
        self.leave_sketch();
    }

    fn remove_empty(&self, entered: Entered, model: &mut Model) {
        let document = model.document();
        let Some(name) = document
            .feature(entered.feature)
            .map(|feature| feature.name.clone())
        else {
            return;
        };
        let dependents = document.dependents_of(&[entered.feature]);
        if !dependents.is_empty() {
            let users = dependents
                .iter()
                .filter_map(|id| document.feature(*id))
                .map(|feature| feature.name.as_str())
                .collect::<Vec<_>>();
            let told = match users.as_slice() {
                [only] => format!("Kept the empty {name}: {only} uses it."),
                [first, rest @ ..] => format!(
                    "Kept the empty {name}: {first} and {} more use it.",
                    rest.len()
                ),
                [] => return,
            };
            model.perform(Action::Inform(Notice::info(told)));
            return;
        }
        let label = format!("Remove the empty {name}");
        let discard = match self.sketch_changes(model) {
            Ok(steps) => discard(&steps, true, label),
            Err(_) => Discard::Apply(model.document().deletion(&[entered.feature], label)),
        };
        let told = match discard {
            Discard::Nothing => return,
            Discard::Undo => format!("Removed the empty {name}; Redo brings it back."),
            Discard::Apply(_) => format!("Removed the empty {name}; Undo brings it back."),
        };
        perform_discard(discard, model);
        model.perform(Action::Inform(Notice::info(told)));
    }

    fn cancel_sketch(&mut self, model: &mut Model) {
        let named = self.entered().and_then(|entered| {
            let feature = model.document().feature(entered.feature)?;
            Some((feature.name.clone(), entered.added))
        });
        let Some((name, added)) = named else {
            model.perform(Action::Inform(Notice::warning(format!(
                "{}: {NO_SKETCH}.",
                Command::CancelSketch.title()
            ))));
            return;
        };
        let label = cancel_title(&name, added);
        let discard = match self.sketch_changes(model) {
            Ok(steps) => discard(&steps, added, label),
            Err(refusal) => {
                model.perform(Action::Inform(Notice::warning(format!(
                    "{label}: {refusal}."
                ))));
                return;
            }
        };
        let told = match (&discard, added) {
            (Discard::Nothing, _) => None,
            (Discard::Undo, _) => Some(format!("Took back the new {name}; Redo brings it back.")),
            (Discard::Apply(_), true) => {
                Some(format!("Took back the new {name}; Undo brings it back."))
            }
            (Discard::Apply(_), false) => Some(format!(
                "Took back the changes to {name}; Undo brings them back."
            )),
        };
        perform_discard(discard, model);
        if let Some(told) = told {
            model.perform(Action::Inform(Notice::info(told)));
        }
        self.leave_sketch();
        self.picking = None;
    }

    fn open_solid(&mut self, feature: FeatureId, model: &Model, since: UndoMark) {
        let document = model.document();
        if !opened_solid(document, feature) || !document.is_active(feature) {
            return;
        }
        self.leave_sketch();
        self.choosing_plane = false;
        self.combining = None;
        if self.solid() == Some(feature) {
            return;
        }
        let added = model
            .undo_steps_since(since)
            .is_some_and(|steps| steps.iter().any(|step| adds(step, feature)));
        self.solid = Some(Opened {
            feature,
            added,
            mark: since,
        });
    }

    fn cancel(&mut self, model: &mut Model) {
        match self.cancelling(model) {
            Ok(steps) => {
                for _ in 0..steps {
                    model.perform(Action::Undo);
                }
                self.solid = None;
                self.picking = None;
            }
            Err(refusal) => {
                let open = self.solid.and_then(|opened| {
                    let feature = model.document().feature(opened.feature)?;
                    Some(cancel_title(&feature.name, opened.added))
                });
                let title = open.unwrap_or_else(|| Command::CancelFeature.title());
                model.perform(Action::Inform(Notice::warning(format!(
                    "{title}: {refusal}."
                ))));
            }
        }
    }

    fn drop_stale_picking(&mut self) {
        if self
            .picking
            .is_some_and(|picking| Some(picking.feature) != self.solid())
        {
            self.picking = None;
        }
    }

    pub fn sync(&mut self, model: &Model) {
        if self.session != model.session() {
            self.session = model.session();
            self.leave_sketch();
            self.solid = None;
            self.choosing_plane = false;
            self.combining = None;
            self.switched = None;
        }
        let document = model.document();
        if let Some(active) = self.active
            && (edited_sketch(document, active.feature).is_none()
                || !document.is_active(active.feature))
        {
            self.leave_sketch();
        }
        if let Some(solid) = self.solid()
            && (!opened_solid(document, solid) || !document.is_active(solid))
        {
            self.solid = None;
        }
        if let Some(combining) = &mut self.combining
            && combining
                .target
                .is_some_and(|target| !document.bodies_standing().contains(&target))
        {
            combining.target = None;
        }
        self.drop_stale_picking();
    }

    fn combine_with(&mut self, body: FeatureId, model: &mut Model, since: UndoMark) {
        let Some(combining) = self.combining else {
            return;
        };
        let target = match combining.target {
            Some(target) if target != body => target,
            Some(_) => {
                self.combining = Some(Combining { target: None });
                return;
            }
            None => {
                self.combining = Some(Combining { target: Some(body) });
                return;
            }
        };
        match combine_tools::pair(model, target, body) {
            Ok(pair) => {
                self.combining = None;
                let (transaction, feature) = combine_tools::create(model.document(), pair);
                model.perform(Action::Apply(transaction));
                self.open_solid(feature, model, since);
            }
            Err(reason) => model.perform(Action::Inform(Notice::warning(format!(
                "{}: {reason}.",
                combine_tools::TITLE
            )))),
        }
    }

    fn create(&mut self, plane: PrincipalPlane, model: &mut Model, since: UndoMark) {
        self.choosing_plane = false;
        let document = model.document();
        let name = next_sketch_name(document);
        let mut transaction = document.transaction(format!("Create {name}"));
        let feature = transaction.add_feature(name, FeatureKind::from(Sketch::new(plane.plane())));
        model.perform(Action::Apply(transaction.finish()));
        self.enter(feature, model, since);
    }

    fn create_on_datum(&mut self, datum: DatumTarget, model: &mut Model, since: UndoMark) {
        match sketch_placement::new_sketch_on_datum(model, datum) {
            Ok((transaction, feature)) => {
                self.choosing_plane = false;
                model.perform(Action::Apply(transaction));
                self.enter(feature, model, since);
            }
            Err(reason) => {
                refuse_new_sketch(model, &reason, EditingCommand::NewSketchOnDatum(datum));
            }
        }
    }

    fn create_on_face(&mut self, face: FaceChoice, model: &mut Model, since: UndoMark) {
        match sketch_placement::new_sketch(model, face) {
            Ok((transaction, feature)) => {
                self.choosing_plane = false;
                model.perform(Action::Apply(transaction));
                self.enter(feature, model, since);
            }
            Err(reason) => refuse_new_sketch(model, &reason, EditingCommand::NewSketchOnFace(face)),
        }
    }

    fn enter(&mut self, feature: FeatureId, model: &Model, since: UndoMark) {
        let document = model.document();
        if edited_sketch(document, feature).is_none() || !document.is_active(feature) {
            return;
        }
        self.choosing_plane = false;
        self.combining = None;
        self.solid = None;
        if self.entered().map(|entered| entered.feature) != Some(feature) {
            let added = model
                .undo_steps_since(since)
                .is_some_and(|steps| steps.iter().any(|step| adds(step, feature)));
            self.entered = Some(Entered {
                feature,
                added,
                mark: since,
            });
        }
        self.active = Some(ActiveSketch {
            feature,
            tool: Tool::Select,
            construction: false,
        });
    }
}

fn switches(command: EditingCommand) -> bool {
    matches!(
        command,
        EditingCommand::NewSketch(_)
            | EditingCommand::NewSketchOnFace(_)
            | EditingCommand::NewSketchOnDatum(_)
            | EditingCommand::Enter(_)
            | EditingCommand::OpenSolid(_)
            | EditingCommand::Pick(_)
    )
}

fn refuse_new_sketch(model: &mut Model, reason: &impl Reason, retry: EditingCommand) {
    let title = Command::NewSketch.title();
    model.perform(match reason.pending() {
        Some(pending) => {
            Action::Queue(Box::new(Queued::new(Rerun::Editing(retry), title, pending)))
        }
        None => Action::Inform(Notice::warning(format!("{title}: {}.", reason.reason()))),
    });
}

fn is_empty(sketch: &Sketch) -> bool {
    sketch.entities().len() == 0 && sketch.constraints().len() == 0
}

fn changes_since(
    model: &Model,
    feature: FeatureId,
    added: bool,
    mark: UndoMark,
) -> Result<Vec<&Transaction>, Option<CancelRefusal>> {
    let document = model.document();
    let name = document
        .feature(feature)
        .map(|feature| feature.name.clone())
        .ok_or(None)?;
    let steps = model
        .undo_steps_since(mark)
        .ok_or_else(|| match model.is_undone_past(mark) {
            true => CancelRefusal::UndoneBeyond {
                feature: name.clone(),
            },
            false => CancelRefusal::HistoryChanged {
                feature: name.clone(),
            },
        })?;
    let owned = parameters_of(document, &steps, feature);
    if let Some(step) = steps
        .iter()
        .find(|step| !(added && adds(step, feature) || changes_only(step, feature, &owned)))
    {
        return Err(Some(CancelRefusal::ChangedElsewhere {
            feature: name,
            step: step.label().to_owned(),
        }));
    }
    Ok(steps)
}

fn discard(steps: &[&Transaction], added: bool, label: String) -> Discard {
    match steps {
        [] => Discard::Nothing,
        [_] if added => Discard::Undo,
        steps => Discard::Apply(Transaction::new(
            label,
            steps
                .iter()
                .flat_map(|step| step.edits().iter().cloned())
                .collect(),
        )),
    }
}

fn perform_discard(discard: Discard, model: &mut Model) {
    match discard {
        Discard::Nothing => {}
        Discard::Undo => model.perform(Action::Undo),
        Discard::Apply(transaction) => model.perform(Action::Apply(transaction)),
    }
}

pub fn cancel_title(name: &str, added: bool) -> String {
    if added {
        format!("Cancel the new {name}")
    } else {
        format!("Cancel the changes to {name}")
    }
}

fn adds(step: &Transaction, feature: FeatureId) -> bool {
    step.edits()
        .iter()
        .any(|edit| matches!(edit, Edit::RemoveFeature { id } if *id == feature))
}

fn changes_only(step: &Transaction, feature: FeatureId, owned: &BTreeSet<ParameterId>) -> bool {
    let touched = step.touched();
    let elsewhere = touched.rollback
        || touched.principal
        || touched.properties
        || touched.views
        || touched.selection_sets
        || touched.configurations;
    !elsewhere
        && touched.features.iter().all(|id| *id == feature)
        && touched.parameters.iter().all(|id| owned.contains(id))
}

fn parameters_of(
    document: &Document,
    steps: &[&Transaction],
    feature: FeatureId,
) -> BTreeSet<ParameterId> {
    let owns =
        |owner: Option<&ParameterOwner>| owner.is_some_and(|owner| owner.feature() == feature);
    let touched = steps.iter().flat_map(|step| step.touched().parameters);
    let live = touched.filter(|id| {
        owns(
            document
                .parameter(*id)
                .and_then(|parameter| parameter.owner.as_ref()),
        )
    });
    let given = steps
        .iter()
        .flat_map(|step| step.edits())
        .filter_map(|edit| match edit {
            Edit::InsertParameter { parameter, .. } if owns(parameter.owner.as_ref()) => {
                Some(parameter.id())
            }
            Edit::SetParameterOwner { id, owner } if owns(owner.as_ref()) => Some(*id),
            _ => None,
        });
    live.chain(given).collect()
}

pub fn edited_sketch(document: &Document, feature: FeatureId) -> Option<&Sketch> {
    document.feature(feature)?.kind.sketch()
}

fn opened_solid(document: &Document, feature: FeatureId) -> bool {
    document
        .feature(feature)
        .is_some_and(|feature| match feature.kind {
            FeatureKind::Solid(_)
            | FeatureKind::Blend(_)
            | FeatureKind::Shell(_)
            | FeatureKind::OffsetFace(_)
            | FeatureKind::DeleteFace(_)
            | FeatureKind::Emboss(_)
            | FeatureKind::MoveFace(_)
            | FeatureKind::Draft(_)
            | FeatureKind::ReplaceFace(_)
            | FeatureKind::SplitFace(_)
            | FeatureKind::Primitive(_)
            | FeatureKind::Pipe(_)
            | FeatureKind::Loft(_)
            | FeatureKind::Coil(_)
            | FeatureKind::Helix(_)
            | FeatureKind::Rib(_)
            | FeatureKind::Combine(_)
            | FeatureKind::Move(_)
            | FeatureKind::Mate(_)
            | FeatureKind::Mirror(_)
            | FeatureKind::Split(_)
            | FeatureKind::Scale(_)
            | FeatureKind::Hole(_)
            | FeatureKind::Pattern(_)
            | FeatureKind::Datum(_)
            | FeatureKind::Thread(_)
            | FeatureKind::Measurement(_)
            | FeatureKind::ReferenceImage(_) => true,
            FeatureKind::Sketch(_) | FeatureKind::Import(_) | FeatureKind::Remove(_) => false,
        })
}

pub fn next_sketch_name(document: &Document) -> String {
    next_feature_name(document, NEW_SKETCH_PREFIX)
}

pub fn next_feature_name(document: &Document, prefix: &str) -> String {
    (1..=document.features().len() + 1)
        .map(|number| format!("{prefix} {number}"))
        .find(|name| document.features().all(|feature| feature.name != *name))
        .unwrap_or_else(|| prefix.to_owned())
}

pub fn next_feature_names(document: &Document, prefix: &str, count: usize) -> Vec<String> {
    (1..)
        .map(|number| format!("{prefix} {number}"))
        .filter(|name| document.features().all(|feature| feature.name != *name))
        .take(count)
        .collect()
}

#[cfg(test)]
mod tests {
    use caditor_geometry::Plane;

    use super::*;

    #[test]
    fn new_sketch_names_fill_the_first_free_number() {
        let mut document = Document::default();
        assert_eq!(next_sketch_name(&document), "Sketch 1");
        let mut transaction = document.transaction("Sketches");
        transaction.add_feature("Sketch 1", FeatureKind::from(Sketch::new(Plane::XY)));
        transaction.add_feature("Sketch 3", FeatureKind::from(Sketch::new(Plane::XY)));
        document.apply(transaction.finish()).unwrap();
        assert_eq!(next_sketch_name(&document), "Sketch 2");
    }
}
