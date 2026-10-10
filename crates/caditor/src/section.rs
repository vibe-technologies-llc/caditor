use std::borrow::Cow;

use caditor_document::{DatumResult, displayed_frame, face_plane};
use caditor_expression::{Dimension, Expression, Unit};
use caditor_geometry::{Plane, Point3, Rotation3};
use caditor_render::{CutFace, MAX_SECTION_PLANES, SectionPlane};

use crate::{
    bodies,
    commands::Reason,
    datum_tools,
    model::Model,
    scene,
    selection::{Pickable, PrincipalPlane, Selection},
    variants::all_variants,
};

const MOST_CONSIDERED: usize = 16;
const OFFSET_STEPS_PER_MILLIMETRE: f64 = 10.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SectionCommand {
    Toggle,
    Add,
    UseSelected,
    Flip,
    Remove,
    SliceSketch,
}

all_variants!(
    SectionCommand: Toggle,
    Add,
    UseSelected,
    Flip,
    Remove,
    SliceSketch
);

impl SectionCommand {
    pub fn id(self) -> &'static str {
        match self {
            Self::Toggle => "view.section",
            Self::Add => "view.section_add",
            Self::UseSelected => "view.section_use_selected",
            Self::Flip => "view.section_flip",
            Self::Remove => "view.section_remove",
            Self::SliceSketch => "sketch.slice_bodies",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Toggle => "Show or hide the section view",
            Self::Add => "Add a section plane",
            Self::UseSelected => "Put the current section plane on the selected plane or face",
            Self::Flip => "Flip the side the current section plane cuts away",
            Self::Remove => "Remove the current section plane",
            Self::SliceSketch => "Slice the bodies at the sketch plane",
        }
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Problem {
    #[error(
        "The plane or face this section starts from is gone or no longer flat. Choose another."
    )]
    BaseGone,
    #[error("The offset or an angle of this section could not be worked out. Enter it again.")]
    ValueUnreadable,
}

const NOTHING_GIVES_A_PLANE: &str =
    "Select a plane, a datum plane, a flat face or a sketch to put the section on.";
const CLOSED: &str = "Open the section view to change its planes";
const NO_PLANE: &str = "There is no section plane. Add one first.";

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Refusal {
    #[error("{NOTHING_GIVES_A_PLANE}")]
    NothingGivesAPlane,
    #[error("{0} of the selected items give a plane. Select one to put the section on.")]
    SeveralGiveAPlane(usize),
    #[error("{0} items are selected. Select one plane or flat face to put the section on.")]
    TooManySelected(usize),
    #[error("{CLOSED}")]
    Closed,
    #[error("{NO_PLANE}")]
    NoPlane,
    #[error("A section shows at most {MAX_SECTION_PLANES} planes at once")]
    TooManyPlanes,
}

impl Reason for Refusal {
    fn reason(&self) -> Cow<'static, str> {
        match self {
            Self::NothingGivesAPlane => Cow::Borrowed(NOTHING_GIVES_A_PLANE),
            Self::Closed => Cow::Borrowed(CLOSED),
            Self::NoPlane => Cow::Borrowed(NO_PLANE),
            Self::SeveralGiveAPlane(_) | Self::TooManySelected(_) | Self::TooManyPlanes => {
                Cow::Owned(self.to_string())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Cut {
    pub base: Pickable,
    pub offset: Expression,
    pub tilt: Expression,
    pub turn: Expression,
    pub flipped: bool,
    pub cut_face: CutFace,
}

impl Cut {
    pub fn through(base: Pickable) -> Self {
        Self {
            base,
            offset: Expression::measure(0.0, Unit::Millimetre),
            tilt: Expression::measure(0.0, Unit::Degree),
            turn: Expression::measure(0.0, Unit::Degree),
            flipped: false,
            cut_face: CutFace::Hatched,
        }
    }

    pub fn centred(model: &Model, base: Pickable, centre: Option<Point3>) -> Self {
        let offset = base_plane(model, base)
            .zip(centre)
            .map_or(0.0, |(plane, centre)| {
                let along = -plane.signed_distance(centre);
                (along * OFFSET_STEPS_PER_MILLIMETRE).round() / OFFSET_STEPS_PER_MILLIMETRE
            });
        Self {
            offset: Expression::measure(without_negative_zero(offset), Unit::Millimetre),
            ..Self::through(base)
        }
    }

    pub fn base_name(&self, model: &Model) -> String {
        self.base.describe(model.document(), model.evaluation())
    }

    pub fn plane(&self, model: &Model) -> Result<SectionPlane, Problem> {
        let base = base_plane(model, self.base).ok_or(Problem::BaseGone)?;
        let offset = value(model, &self.offset, Dimension::LENGTH)?;
        let tilt = value(model, &self.tilt, Dimension::ANGLE)?;
        let turn = value(model, &self.turn, Dimension::ANGLE)?;
        let tilted = Rotation3::from_axis_angle(base.x_axis(), tilt.to_radians());
        let x_axis = tilted * base.x_axis();
        let y_axis = tilted * base.y_axis();
        let turned = Rotation3::from_axis_angle(y_axis, turn.to_radians());
        let normal = turned * (tilted * base.normal());
        let origin = base.origin() - normal * offset;
        let facing = if self.flipped { -normal } else { normal };
        let plane = Plane::with_x_axis(origin, facing, turned * x_axis)
            .or_else(|| Plane::new(origin, facing))
            .ok_or(Problem::BaseGone)?;
        Ok(SectionPlane {
            plane,
            cut_face: self.cut_face,
        })
    }
}

fn without_negative_zero(value: f64) -> f64 {
    value + 0.0
}

fn value(model: &Model, expression: &Expression, dimension: Dimension) -> Result<f64, Problem> {
    let quantity = model
        .parameters()
        .evaluate_expression(expression)
        .map_err(|_| Problem::ValueUnreadable)?;
    if quantity.dimension != dimension || !quantity.value.is_finite() {
        return Err(Problem::ValueUnreadable);
    }
    Ok(quantity.value)
}

pub fn base_plane(model: &Model, pickable: Pickable) -> Option<Plane> {
    let document = model.document();
    let evaluation = model.evaluation();
    match pickable {
        Pickable::Plane(plane) => Some(plane.plane()),
        Pickable::Datum(feature) => match datum_tools::result(evaluation, feature)? {
            DatumResult::Plane(plane) => Some(plane),
            DatumResult::Axis(_) | DatumResult::Point(_) | DatumResult::Frame(_) => None,
        },
        Pickable::FramePlane { feature, plane } => {
            plane.in_frame(&displayed_frame(evaluation, feature)?)
        }
        Pickable::Face { body, face } => {
            let shown = bodies::shown(evaluation, body)?;
            face_plane(&shown.solid, bodies::find_face(shown, face)?)
        }
        Pickable::SketchEntity { feature, .. } | Pickable::SketchRegion { feature, .. } => {
            scene::sketch_plane(document, evaluation, feature)
        }
        Pickable::Origin
        | Pickable::Axis(_)
        | Pickable::SketchConstraint { .. }
        | Pickable::Edge { .. }
        | Pickable::Vertex { .. }
        | Pickable::Region { .. }
        | Pickable::BlendEdge { .. }
        | Pickable::ShellFace { .. }
        | Pickable::FrameAxis { .. }
        | Pickable::FeatureValue { .. }
        | Pickable::CentreOfMass(_) => None,
    }
}

pub fn base_from(model: &Model, selection: &Selection) -> Result<Pickable, Refusal> {
    if selection.len() > MOST_CONSIDERED {
        return Err(Refusal::TooManySelected(selection.len()));
    }
    let mut giving: Vec<Pickable> = selection
        .in_pick_order()
        .into_iter()
        .filter(|pickable| base_plane(model, *pickable).is_some())
        .collect();
    match giving.len() {
        0 => Err(Refusal::NothingGivesAPlane),
        1 => giving.pop().ok_or(Refusal::NothingGivesAPlane),
        several => Err(Refusal::SeveralGiveAPlane(several)),
    }
}

#[derive(Default)]
pub struct SectionTool {
    pub open: bool,
    pub cuts: Vec<Cut>,
    pub current: usize,
}

impl SectionTool {
    pub fn toggle(&mut self, model: &Model, centre: Option<Point3>) {
        self.open = !self.open;
        if self.open && self.cuts.is_empty() {
            self.add(model, centre);
        }
    }

    pub fn can_add(&self) -> Result<(), Refusal> {
        if self.cuts.len() >= MAX_SECTION_PLANES {
            Err(Refusal::TooManyPlanes)
        } else {
            Ok(())
        }
    }

    pub fn add(&mut self, model: &Model, centre: Option<Point3>) {
        if self.can_add().is_err() {
            return;
        }
        let unused = [PrincipalPlane::Xz, PrincipalPlane::Yz, PrincipalPlane::Xy]
            .into_iter()
            .map(Pickable::Plane)
            .find(|base| self.cuts.iter().all(|cut| cut.base != *base))
            .unwrap_or(Pickable::Plane(PrincipalPlane::Xz));
        self.cuts.push(Cut::centred(model, unused, centre));
        self.current = self.cuts.len() - 1;
        self.open = true;
    }

    pub fn current_cut(&self) -> Result<usize, Refusal> {
        if !self.open {
            return Err(Refusal::Closed);
        }
        if self.cuts.is_empty() {
            return Err(Refusal::NoPlane);
        }
        Ok(self.current.min(self.cuts.len() - 1))
    }

    pub fn current_mut(&mut self) -> Option<&mut Cut> {
        let index = self.current_cut().ok()?;
        self.cuts.get_mut(index)
    }

    pub fn remove(&mut self, index: usize) {
        if index < self.cuts.len() {
            self.cuts.remove(index);
        }
        self.current = self.current.min(self.cuts.len().saturating_sub(1));
    }

    pub fn forget(&mut self) {
        let open = self.open;
        *self = Self {
            open,
            ..Self::default()
        };
    }

    pub fn planes(&self, model: &Model) -> Vec<SectionPlane> {
        if !self.open {
            return Vec::new();
        }
        self.cuts
            .iter()
            .filter_map(|cut| cut.plane(model).ok())
            .take(MAX_SECTION_PLANES)
            .collect()
    }
}

pub fn sketch_slice(plane: Plane) -> SectionPlane {
    SectionPlane {
        plane,
        cut_face: CutFace::Hatched,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use caditor_document::Document;
    use caditor_file::StorageConfig;
    use caditor_geometry::{Point3, Vector3};

    use super::*;
    use crate::model::Services;

    fn empty_model() -> Model {
        Model::new(
            Document::default(),
            Services {
                make_waker: Box::new(|| Box::new(|| {})),
                storage: StorageConfig::default(),
                panic_flush: Arc::default(),
            },
        )
    }

    fn close(a: Vector3, b: Vector3) -> bool {
        a.distance(b) < 1e-9
    }

    #[test]
    fn a_cut_moves_into_the_side_it_keeps_turns_about_its_axes_and_flips_in_place() {
        let model = empty_model();
        let mut cut = Cut::through(Pickable::Plane(PrincipalPlane::Xy));

        let level = cut.plane(&model).unwrap().plane;
        cut.offset = Expression::measure(5.0, Unit::Millimetre);
        let lowered = cut.plane(&model).unwrap().plane;
        cut.flipped = true;
        let flipped = cut.plane(&model).unwrap().plane;
        cut.flipped = false;
        cut.offset = Expression::measure(0.0, Unit::Millimetre);
        cut.tilt = Expression::measure(90.0, Unit::Degree);
        let tilted = cut.plane(&model).unwrap().plane;
        cut.tilt = Expression::measure(0.0, Unit::Degree);
        cut.turn = Expression::measure(90.0, Unit::Degree);
        let turned = cut.plane(&model).unwrap().plane;

        assert!(close(level.normal(), Vector3::Z));
        assert!(close(level.origin(), Point3::ZERO));
        assert!(close(lowered.origin(), Point3::new(0.0, 0.0, -5.0)));
        assert!(close(flipped.normal(), Vector3::NEG_Z));
        assert!(close(flipped.origin(), lowered.origin()));
        assert!(close(tilted.normal(), Vector3::NEG_Y));
        assert!(close(turned.normal(), Vector3::X));
    }

    #[test]
    fn a_section_keeps_at_most_the_planes_the_renderer_draws_and_shows_none_while_closed() {
        let model = empty_model();
        let mut tool = SectionTool::default();

        assert!(tool.planes(&model).is_empty());
        tool.toggle(&model, Some(Point3::new(0.0, 12.34, 0.0)));
        assert_eq!(tool.cuts.len(), 1);
        assert!(close(
            tool.planes(&model)[0].plane.origin(),
            Point3::new(0.0, 12.3, 0.0)
        ));
        for _ in 0..MAX_SECTION_PLANES + 2 {
            tool.add(&model, None);
        }
        assert_eq!(tool.cuts.len(), MAX_SECTION_PLANES);
        assert_eq!(tool.can_add(), Err(Refusal::TooManyPlanes));
        assert_eq!(tool.planes(&model).len(), MAX_SECTION_PLANES);
        assert_eq!(tool.current, MAX_SECTION_PLANES - 1);

        tool.remove(tool.current);
        assert_eq!(tool.current, MAX_SECTION_PLANES - 2);
        tool.cuts.clear();
        assert_eq!(tool.current_cut(), Err(Refusal::NoPlane));
        tool.open = false;
        assert_eq!(tool.current_cut(), Err(Refusal::Closed));
    }
}
