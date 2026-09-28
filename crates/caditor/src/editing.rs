use caditor_document::{Document, FeatureId, FeatureKind};
use caditor_sketch::Sketch;
use egui::Key;

use crate::{
    model::{Action, Model},
    selection::PrincipalPlane,
    sketch_placement::{self, FaceChoice},
};

const NEW_SKETCH_PREFIX: &str = "Sketch";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Tool {
    #[default]
    Select,
    Point,
    Line,
    Rectangle,
    Circle,
    Arc,
    Spline,
}

impl Tool {
    pub const ALL: [Self; 7] = [
        Self::Select,
        Self::Point,
        Self::Line,
        Self::Rectangle,
        Self::Circle,
        Self::Arc,
        Self::Spline,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Select => "Select",
            Self::Point => "Point",
            Self::Line => "Line",
            Self::Rectangle => "Rectangle",
            Self::Circle => "Circle",
            Self::Arc => "Arc",
            Self::Spline => "Spline",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Select => "⬉",
            Self::Point => "•",
            Self::Line => "∕",
            Self::Rectangle => "☐",
            Self::Circle => "○",
            Self::Arc => "↺",
            Self::Spline => "∫",
        }
    }

    pub fn button_text(self) -> String {
        format!("{} {}", self.icon(), self.label())
    }

    pub fn key(self) -> Option<Key> {
        match self {
            Self::Select => None,
            Self::Point => Some(Key::P),
            Self::Line => Some(Key::L),
            Self::Rectangle => Some(Key::R),
            Self::Circle => Some(Key::C),
            Self::Arc => Some(Key::A),
            Self::Spline => Some(Key::S),
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Select => "Click geometry to select it for constraints or deletion",
            Self::Point => "Place points",
            Self::Line => "Draw connected lines, one click per corner",
            Self::Rectangle => "Draw a rectangle from two opposite corners",
            Self::Circle => "Draw a circle from its centre and a point on it",
            Self::Arc => "Draw an arc from its centre, start and end",
            Self::Spline => "Draw a smooth curve through control points",
        }
    }

    pub fn draws(self) -> bool {
        match self {
            Self::Select => false,
            Self::Point
            | Self::Line
            | Self::Rectangle
            | Self::Circle
            | Self::Arc
            | Self::Spline => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveSketch {
    pub feature: FeatureId,
    pub tool: Tool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EditingCommand {
    NewSketch(Option<PrincipalPlane>),
    NewSketchOnFace(FaceChoice),
    CancelNewSketch,
    Enter(FeatureId),
    Finish,
    SetTool(Tool),
    OpenSolid(FeatureId),
    CloseSolid,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Context {
    pub sketch: Option<FeatureId>,
    pub solid: Option<FeatureId>,
}

#[derive(Debug, Clone, Default)]
pub struct SketchEditing {
    active: Option<ActiveSketch>,
    solid: Option<FeatureId>,
    choosing_plane: bool,
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

    pub fn context(&self) -> Context {
        Context {
            sketch: self.feature(),
            solid: self.solid,
        }
    }

    pub fn is_choosing_plane(&self) -> bool {
        self.choosing_plane
    }

    #[cfg(test)]
    pub fn editing(feature: FeatureId) -> Self {
        Self {
            active: Some(ActiveSketch {
                feature,
                tool: Tool::Select,
            }),
            ..Self::default()
        }
    }

    pub fn perform(&mut self, command: EditingCommand, model: &mut Model) {
        self.sync(model);
        match command {
            EditingCommand::NewSketch(Some(plane)) => self.create(plane, model),
            EditingCommand::NewSketchOnFace(face) => self.create_on_face(face, model),
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
            EditingCommand::OpenSolid(feature) => {
                if opened_solid(model.document(), feature) {
                    self.active = None;
                    self.choosing_plane = false;
                    self.solid = Some(feature);
                }
            }
            EditingCommand::CloseSolid => self.solid = None,
        }
    }

    pub fn sync(&mut self, model: &Model) {
        if self.session != model.session() {
            self.session = model.session();
            self.active = None;
            self.solid = None;
            self.choosing_plane = false;
        }
        if let Some(active) = self.active
            && edited_sketch(model.document(), active.feature).is_none()
        {
            self.active = None;
        }
        if let Some(solid) = self.solid
            && !opened_solid(model.document(), solid)
        {
            self.solid = None;
        }
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

    fn create_on_face(&mut self, face: FaceChoice, model: &mut Model) {
        let Some((transaction, feature)) = sketch_placement::new_sketch(model, face) else {
            return;
        };
        self.choosing_plane = false;
        model.perform(Action::Apply(transaction));
        self.enter(feature, model.document());
    }

    fn enter(&mut self, feature: FeatureId, document: &Document) {
        if edited_sketch(document, feature).is_some() {
            self.choosing_plane = false;
            self.solid = None;
            self.active = Some(ActiveSketch {
                feature,
                tool: Tool::Select,
            });
        }
    }
}

pub fn edited_sketch(document: &Document, feature: FeatureId) -> Option<&Sketch> {
    document.feature(feature)?.kind.sketch()
}

fn opened_solid(document: &Document, feature: FeatureId) -> bool {
    document.feature(feature).is_some_and(|feature| {
        feature.kind.solid().is_some()
            || feature.kind.blend().is_some()
            || feature.kind.shell().is_some()
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
