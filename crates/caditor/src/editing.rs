use caditor_document::{Document, FeatureId, FeatureKind};
use caditor_sketch::Sketch;

use crate::{
    commands::Command,
    model::{Action, Model, Notice},
    reference_picking::Picking,
    selection::{Pickable, PrincipalPlane},
    shape_modes::{ShapeMode, ShapeModes},
    sketch_placement::{self, FaceChoice},
    variants::all_variants,
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
    Trim,
    Extend,
    Offset,
    Mirror,
    RectangularPattern,
    CircularPattern,
    TangentCircle,
    Fillet,
    Chamfer,
    Project,
    Dimension,
}

all_variants!(Tool: Select, Point, Line, Rectangle, Circle, Arc, ThreePointArc, TangentArc, Slot, Polygon, Spline, Trim, Extend, Offset, Mirror, RectangularPattern, CircularPattern, TangentCircle, Fillet, Chamfer, Project, Dimension);

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
            Self::Fillet => "Sketch fillet",
            Self::Chamfer => "Sketch chamfer",
            Self::Project => "Project",
            Self::Dimension => "Smart dimension",
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
            Self::Rectangle => "Draw a rectangle from two opposite corners",
            Self::Circle => "Draw a circle from its centre and a point on it",
            Self::Arc => "Draw an arc from its centre, start and end",
            Self::ThreePointArc => "Draw an arc from its start and end through a third point",
            Self::TangentArc => {
                "Draw arcs that continue smoothly from the end of a line, arc or spline"
            }
            Self::Slot => "Draw a slot from the centres of its round ends and its width",
            Self::Polygon => "Draw a regular polygon from its centre and a corner",
            Self::Spline => "Draw a smooth curve through control points",
            Self::Trim => {
                "Click a piece of a line, circle or arc to cut it away up to the curves crossing \
                 it, or drag across several pieces"
            }
            Self::Extend => {
                "Click near the end of a line or arc to lengthen it to the next curve in its way"
            }
            Self::Offset => {
                "Copy the selected chain of lines, arcs or circles at a distance to one side, kept \
                 at that distance as the original changes"
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
            Self::Fillet => {
                "Round the corner where two lines or arcs meet with an arc tangent to both"
            }
            Self::Chamfer => {
                "Cut the corner where two lines or arcs meet with a line the same distance from \
                 it on both"
            }
            Self::Project => {
                "Click an edge, corner or face of a body, or a curve of another sketch, to bring \
                 it into this sketch; it follows the original as the model changes"
            }
            Self::Dimension => {
                "Click the geometry to dimension: a line for its length, a circle or arc for its \
                 size, or two items for the distance or angle between them"
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
            | Self::Fillet
            | Self::Chamfer
            | Self::Project
            | Self::Dimension => false,
            Self::Point
            | Self::Line
            | Self::Rectangle
            | Self::Circle
            | Self::Arc
            | Self::ThreePointArc
            | Self::TangentArc
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
                | Self::Fillet
                | Self::Chamfer
        )
    }

    pub fn projects(self) -> bool {
        self == Self::Project
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
    NewSketchOnDatum(FeatureId),
    CancelNewSketch,
    Enter(FeatureId),
    Finish,
    SetTool(Tool),
    SetMode(ShapeMode),
    DrawConstruction(bool),
    OpenSolid(FeatureId),
    CloseSolid,
    Pick(Picking),
    HoldPicked(Pickable),
    StopPicking,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Context {
    pub sketch: Option<FeatureId>,
    pub solid: Option<FeatureId>,
    pub choosing_plane: bool,
    pub projecting: bool,
    pub selecting: bool,
    pub choosing_in_view: bool,
}

impl Context {
    pub fn picks_shown_regions(self) -> bool {
        self.sketch.is_none()
            && self.solid.is_none()
            && !self.choosing_plane
            && !self.choosing_in_view
    }
}

#[derive(Debug, Clone, Default)]
pub struct SketchEditing {
    active: Option<ActiveSketch>,
    solid: Option<FeatureId>,
    picking: Option<Picking>,
    choosing_plane: bool,
    modes: ShapeModes,
    session: u64,
}

impl SketchEditing {
    pub fn active(&self) -> Option<ActiveSketch> {
        self.active
    }

    pub fn feature(&self) -> Option<FeatureId> {
        self.active.map(|active| active.feature)
    }

    pub fn solid(&self) -> Option<FeatureId> {
        self.solid
    }

    pub fn modes(&self) -> ShapeModes {
        self.modes
    }

    pub fn context(&self) -> Context {
        Context {
            sketch: self.feature(),
            solid: self.solid,
            choosing_plane: self.choosing_plane,
            projecting: self.active.is_some_and(|active| active.tool.projects()),
            selecting: self
                .active
                .is_some_and(|active| active.tool == Tool::Select),
            choosing_in_view: self.picking.is_some(),
        }
    }

    pub fn is_choosing_plane(&self) -> bool {
        self.choosing_plane
    }

    pub fn picking(&self) -> Option<Picking> {
        self.picking
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
        self.sync(model);
        match command {
            EditingCommand::NewSketch(Some(plane)) => self.create(plane, model),
            EditingCommand::NewSketchOnFace(face) => self.create_on_face(face, model),
            EditingCommand::NewSketchOnDatum(datum) => self.create_on_datum(datum, model),
            EditingCommand::NewSketch(None) => {
                self.active = None;
                self.solid = None;
                self.choosing_plane = true;
            }
            EditingCommand::CancelNewSketch => self.choosing_plane = false,
            EditingCommand::Enter(feature) => self.enter(feature, model.document()),
            EditingCommand::Finish => self.active = None,
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
            EditingCommand::OpenSolid(feature) => self.open_solid(feature, model.document()),
            EditingCommand::CloseSolid => self.solid = None,
            EditingCommand::Pick(picking) => {
                self.open_solid(picking.feature, model.document());
                if self.solid == Some(picking.feature) {
                    self.picking = Some(picking);
                }
            }
            EditingCommand::HoldPicked(pickable) => {
                if let Some(picking) = &mut self.picking {
                    picking.hold(pickable);
                }
            }
            EditingCommand::StopPicking => self.picking = None,
        }
        self.drop_stale_picking();
    }

    fn open_solid(&mut self, feature: FeatureId, document: &Document) {
        if opened_solid(document, feature) && document.is_active(feature) {
            self.active = None;
            self.choosing_plane = false;
            self.solid = Some(feature);
        }
    }

    fn drop_stale_picking(&mut self) {
        if self
            .picking
            .is_some_and(|picking| Some(picking.feature) != self.solid)
        {
            self.picking = None;
        }
    }

    pub fn sync(&mut self, model: &Model) {
        if self.session != model.session() {
            self.session = model.session();
            self.active = None;
            self.solid = None;
            self.choosing_plane = false;
        }
        let document = model.document();
        if let Some(active) = self.active
            && (edited_sketch(document, active.feature).is_none()
                || !document.is_active(active.feature))
        {
            self.active = None;
        }
        if let Some(solid) = self.solid
            && (!opened_solid(document, solid) || !document.is_active(solid))
        {
            self.solid = None;
        }
        self.drop_stale_picking();
    }

    fn create(&mut self, plane: PrincipalPlane, model: &mut Model) {
        self.choosing_plane = false;
        let document = model.document();
        let name = next_sketch_name(document);
        let mut transaction = document.transaction(format!("Create {name}"));
        let feature = transaction.add_feature(name, FeatureKind::from(Sketch::new(plane.plane())));
        model.perform(Action::Apply(transaction.finish()));
        self.enter(feature, model.document());
    }

    fn create_on_datum(&mut self, datum: FeatureId, model: &mut Model) {
        match sketch_placement::new_sketch_on_datum(model, datum) {
            Ok((transaction, feature)) => {
                self.choosing_plane = false;
                model.perform(Action::Apply(transaction));
                self.enter(feature, model.document());
            }
            Err(reason) => refuse_new_sketch(model, &reason),
        }
    }

    fn create_on_face(&mut self, face: FaceChoice, model: &mut Model) {
        match sketch_placement::new_sketch(model, face) {
            Ok((transaction, feature)) => {
                self.choosing_plane = false;
                model.perform(Action::Apply(transaction));
                self.enter(feature, model.document());
            }
            Err(reason) => refuse_new_sketch(model, reason),
        }
    }

    fn enter(&mut self, feature: FeatureId, document: &Document) {
        if edited_sketch(document, feature).is_some() && document.is_active(feature) {
            self.choosing_plane = false;
            self.solid = None;
            self.active = Some(ActiveSketch {
                feature,
                tool: Tool::Select,
                construction: false,
            });
        }
    }
}

fn refuse_new_sketch(model: &mut Model, reason: &str) {
    model.perform(Action::Inform(Notice::info(format!(
        "{}: {reason}.",
        Command::NewSketch.title()
    ))));
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
            | FeatureKind::Combine(_)
            | FeatureKind::Move(_)
            | FeatureKind::Mirror(_)
            | FeatureKind::Split(_)
            | FeatureKind::Scale(_)
            | FeatureKind::Hole(_)
            | FeatureKind::Pattern(_)
            | FeatureKind::Datum(_)
            | FeatureKind::Thread(_) => true,
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
