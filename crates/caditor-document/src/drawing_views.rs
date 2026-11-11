use std::{
    panic::{self, AssertUnwindSafe},
    sync::Arc,
};

use caditor_expression::EvalError;
use caditor_geometry::{Aabb, Aabb2, Plane, Point2, Vector2, Vector3};
use caditor_kernel::{
    BooleanError, HiddenLineError, HiddenLineView, MeshQuality, Solid, ViewEdgeKind, ViewPiece,
    ViewSource, Visibility, hidden_lines,
};
use thiserror::Error;

use crate::{
    document::{Document, FeatureId},
    drawing_sheet::PlacedView,
    drawings::{Drawing, DrawnView, StandardView},
    recompute::{Evaluation, FeatureResult},
    split::HalfSpaceError,
};

#[derive(Debug, Clone, PartialEq, Error)]
pub enum DrawingViewError {
    #[error("there is no body to draw")]
    NoBodies,
    #[error("working out the lines was stopped")]
    Cancelled,
    #[error("its lines could not be worked out from these bodies")]
    Lines(HiddenLineError),
    #[error("working out its lines failed unexpectedly")]
    Panicked,
    #[error("the cutting plane passes through none of the bodies")]
    PlaneMisses,
    #[error("the cutting plane could not be placed across a body: {0}")]
    CuttingSpace(HalfSpaceError),
    #[error("a body could not be cut along the plane: {0}")]
    Cut(BooleanError),
    #[error("where it cuts could not be worked out: {0}")]
    Offset(EvalError),
    #[error(
        "the plane is more than {} mm from the origin",
        crate::drawings::MAX_SECTION_OFFSET
    )]
    OffsetOutOfRange,
    #[error("the view it enlarges could not be drawn")]
    ParentFailed,
    #[error("the view it enlarges is not on the sheet")]
    ParentNotDrawn,
}

impl From<HiddenLineError> for DrawingViewError {
    fn from(error: HiddenLineError) -> Self {
        match error {
            HiddenLineError::Cancelled(_) => Self::Cancelled,
            other => Self::Lines(other),
        }
    }
}

impl StandardView {
    pub fn look_and_up(self) -> (Vector3, Vector3) {
        match self {
            Self::Front => (Vector3::Y, Vector3::Z),
            Self::Top => (-Vector3::Z, Vector3::Y),
            Self::Right => (-Vector3::X, Vector3::Z),
            Self::Left => (Vector3::X, Vector3::Z),
            Self::Isometric => (Vector3::new(-1.0, 1.0, -1.0), Vector3::Z),
        }
    }

    pub fn hidden_line_view(self) -> Result<HiddenLineView, HiddenLineError> {
        let (look, up) = self.look_and_up();
        HiddenLineView::new(look, up)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Disc {
    pub centre: Point2,
    pub radius: f64,
}

impl Disc {
    pub fn bounds(&self) -> Aabb2 {
        let reach = Vector2::splat(self.radius);
        Aabb2::from_point(self.centre - reach).including(self.centre + reach)
    }

    pub fn contains(&self, point: Point2) -> bool {
        (point - self.centre).length_squared() <= self.radius * self.radius
    }

    pub fn clip_segment(&self, from: Point2, to: Point2) -> Option<(Point2, Point2)> {
        let along = to - from;
        let offset = from - self.centre;
        let a = along.dot(along);
        let b = 2.0 * along.dot(offset);
        let c = offset.dot(offset) - self.radius * self.radius;
        let discriminant = b * b - 4.0 * a * c;
        if a <= 0.0 || discriminant <= 0.0 {
            return None;
        }
        let root = discriminant.sqrt();
        let entering = ((-b - root) / (2.0 * a)).max(0.0);
        let leaving = ((-b + root) / (2.0 * a)).min(1.0);
        (entering < leaving).then(|| (from + along * entering, from + along * leaving))
    }

    pub fn scaled_onto(&self, placed: &PlacedView) -> Self {
        Self {
            centre: placed.on_sheet(self.centre),
            radius: self.radius * placed.scale,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HatchRegion {
    pub edges: Vec<(Point2, Point2)>,
    pub angle: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SectionLines {
    pub cutting: Plane,
    pub hatching: Vec<HatchRegion>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ViewLines {
    pub view: DrawnView,
    pub pieces: Vec<ViewPiece>,
    pub bounds: Option<Aabb2>,
    pub undecided: usize,
    pub section: Option<SectionLines>,
    pub clip: Option<Disc>,
}

impl ViewLines {
    pub fn drawn_pieces(&self, hidden: bool) -> impl Iterator<Item = &ViewPiece> {
        let hidden = hidden && self.section.is_none();
        self.pieces
            .iter()
            .filter(move |piece| hidden || piece.visibility == Visibility::Visible)
    }

    pub fn hatching(&self) -> &[HatchRegion] {
        self.section
            .as_ref()
            .map_or(&[], |section| section.hatching.as_slice())
    }
}

fn drawn(piece: &ViewPiece) -> bool {
    match piece.source {
        ViewSource::Edge { kind, .. } => kind == ViewEdgeKind::Sharp,
        ViewSource::Silhouette { .. } => true,
    }
}

pub(crate) fn scene_extent(solids: &[&Solid]) -> f64 {
    solids
        .iter()
        .filter_map(|solid| solid.bounding_box())
        .reduce(Aabb::union)
        .map(|bounds| bounds.diagonal())
        .filter(|diagonal| diagonal.is_finite())
        .unwrap_or(0.0)
}

pub(crate) fn lines_seen(
    view: DrawnView,
    looking: &HiddenLineView,
    solids: &[&Solid],
) -> Result<ViewLines, DrawingViewError> {
    let tolerance = MeshQuality::SMOOTH.tolerance(scene_extent(solids));
    let lines = panic::catch_unwind(AssertUnwindSafe(|| {
        hidden_lines(solids, looking, &tolerance)
    }))
    .map_err(|_| DrawingViewError::Panicked)??;
    let pieces: Vec<ViewPiece> = lines.pieces.into_iter().filter(drawn).collect();
    let bounds = pieces
        .iter()
        .map(|piece| piece.curve.bounding_box(piece.range))
        .reduce(Aabb2::union);
    Ok(ViewLines {
        view,
        pieces,
        bounds,
        undecided: lines.undecided,
        section: None,
        clip: None,
    })
}

pub fn drawing_view(view: StandardView, solids: &[&Solid]) -> Result<ViewLines, DrawingViewError> {
    if solids.is_empty() {
        return Err(DrawingViewError::NoBodies);
    }
    let looking = view.hidden_line_view()?;
    lines_seen(DrawnView::Standard(view), &looking, solids)
}

#[derive(Debug, Clone, Default)]
pub struct DrawnBodies {
    pub results: Vec<(FeatureId, Arc<FeatureResult>)>,
    pub missing: Vec<FeatureId>,
}

impl DrawnBodies {
    pub fn solids(&self) -> Vec<&Solid> {
        self.results
            .iter()
            .filter_map(|(_, result)| result.solid())
            .map(|result| &result.solid)
            .collect()
    }
}

fn solid_result(evaluation: &Evaluation, body: FeatureId) -> Option<Arc<FeatureResult>> {
    let result = evaluation.body_result(body)?;
    result.solid()?;
    Some(Arc::clone(result))
}

pub fn drawn_bodies(
    document: &Document,
    evaluation: &Evaluation,
    drawing: &Drawing,
) -> DrawnBodies {
    let mut drawn = DrawnBodies::default();
    if drawing.bodies.is_empty() {
        drawn.results = evaluation
            .bodies()
            .map(|(body, _)| body)
            .filter(|body| document.body_maker(*body).is_some() && !document.body_hidden(*body))
            .filter_map(|body| Some((body, solid_result(evaluation, body)?)))
            .collect();
        return drawn;
    }
    for body in &drawing.bodies {
        match solid_result(evaluation, *body) {
            Some(result) => drawn.results.push((*body, result)),
            None => drawn.missing.push(*body),
        }
    }
    drawn
}
