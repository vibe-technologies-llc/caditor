use std::{borrow::Cow, collections::BTreeSet};

use caditor_document::{
    AxisReference, BodyOperation, Document, Edit, Evaluation, Extrude, ExtrudeExtent, Feature,
    FeatureId, FeatureKind, FeatureResult, ParameterValues, Pieces, RegionChoice, Revolve,
    RevolveAxis, RevolveExtent, SketchAttachment, SketchFeature, SketchRegion, SolidFeature,
    SolidStart, Transaction, Wall, describe_axis, face_plane, profile_curve, sketch_regions,
};
use caditor_expression::{Expression, Unit};
use caditor_geometry::{Plane, Point2};
use caditor_kernel::{ANGULAR_RESOLUTION, LINEAR_RESOLUTION, RegionKey, WallSide};
use caditor_sketch::{Entity, EntityId, Reference, Sketch};

use crate::{
    bodies::{self, FaceKey},
    body_reach, body_selection,
    commands::Reason,
    editing::{self, EditingCommand, SketchEditing},
    extrusion_detection,
    field::{self, CheckError},
    last_values::{Remembered, Starts},
    model::{Action, Model, Notice},
    projecting, scene,
    selection::{self, Pickable, Selection},
    sketch_placement::{self, FaceChoice},
    solid_panel, visibility,
};

pub const DEFAULT_DISTANCE: f64 = 10.0;
pub const DEFAULT_PARTIAL_ANGLE: f64 = 180.0;
pub const DEFAULT_BACKWARD_ANGLE: f64 = 30.0;
pub const DEFAULT_WALL_THICKNESS: f64 = 1.0;
const OUTLINE_SEGMENT_ANGLE: f64 = 0.02;
const OUTLINE_GAP: f64 = 1e-6;
pub const NOTHING_TO_EXTRUDE: &str =
    "Select flat faces of one plane of a body to extrude them, or the curves of a sketch";
pub const SEVERAL_SKETCHES: &str =
    "Curves of several sketches are selected; select the curves of one sketch only";
pub const SEVERAL_BODIES: &str =
    "Faces of several bodies are selected; select faces of the one body to add to";
pub const NOTHING_TO_REVOLVE: &str = "Select the curves of a sketch or flat faces of a body to revolve, with the axis to turn them about";

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SweepSourceError {
    #[error("{}", SEVERAL_SKETCHES)]
    SeveralSketches,
    #[error("{}", SEVERAL_BODIES)]
    SeveralBodies,
}

impl Reason for SweepSourceError {
    fn reason(&self) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum FaceSweepError {
    #[error("The selected face is curved; only a flat face can be {}", .sweep.passive())]
    NotFlat { sweep: Sweep },
    #[error("The selected faces do not lie in one plane; select flat faces of one plane only")]
    NotInOnePlane,
    #[error("Faces of several bodies are selected; select faces of one body only")]
    SeveralBodies,
    #[error("Select the axis, straight edge or round face to revolve the face about as well")]
    NoAxis,
    #[error("The selected faces have no edges to sweep them by")]
    NoOutline,
    #[error("{0}")]
    Attachment(String),
}

impl Reason for FaceSweepError {
    fn reason(&self) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RegionError {
    #[error("{name} is not an extrusion or a revolve")]
    NotASweep { name: String },
    #[error("{name} has no region chosen")]
    NoneChosen { name: String },
    #[error("{name} takes no regions of a sketch")]
    TakesNoRegions { name: String },
    #[error("{name} is a thin wall, which follows every curve of its sketch")]
    ThinWall { name: String },
    #[error("{name} already takes every region of its sketch")]
    AlreadyEvery { name: String },
    #[error("Only a sketch has regions to take")]
    NotASketch { feature: FeatureId },
    #[error(transparent)]
    Checked(#[from] CheckError),
}

impl From<RegionError> for String {
    fn from(error: RegionError) -> Self {
        error.to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sweep {
    Extrude,
    Revolve,
}

impl Sweep {
    pub const ALL: [Self; 2] = [Self::Extrude, Self::Revolve];

    pub fn label(self) -> &'static str {
        match self {
            Self::Extrude => "Extrude",
            Self::Revolve => "Revolve",
        }
    }

    fn passive(self) -> &'static str {
        match self {
            Self::Extrude => "extruded",
            Self::Revolve => "revolved",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SweepSource {
    pub sketch: FeatureId,
    pub axis: Option<RevolveAxis>,
    pub regions: RegionChoice,
    pub body: Option<FeatureId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Guess {
    Never,
    Sweep,
    Drill,
}

impl Guess {
    pub fn sweep_when(may_guess: bool) -> Self {
        if may_guess { Self::Sweep } else { Self::Never }
    }
}

pub fn sweep_source(
    document: &Document,
    evaluation: &Evaluation,
    selection: &Selection,
    editing: &SketchEditing,
    guess: Guess,
) -> Result<Option<SweepSource>, SweepSourceError> {
    let picked = selection.in_pick_order();
    let mut selected: Vec<FeatureId> = Vec::new();
    for pickable in &picked {
        if let Pickable::SketchEntity { feature, .. } | Pickable::SketchRegion { feature, .. } =
            *pickable
            && !selected.contains(&feature)
        {
            selected.push(feature);
        }
    }
    let (sketch, regions) = match (editing.feature(), selected.as_slice()) {
        (Some(edited), _) => (edited, RegionChoice::All),
        (None, [sketch]) => (*sketch, RegionChoice::All),
        (None, [_, _, ..]) => return Err(SweepSourceError::SeveralSketches),
        (None, []) => match guessed_sketch(document, evaluation, editing, guess) {
            Some(guessed) => (guessed, regions_left(document, evaluation, guessed)),
            None => return Ok(None),
        },
    };
    let Some(definition) = editing::edited_sketch(document, sketch) else {
        return Ok(None);
    };
    let axis = picked.iter().rev().find_map(|pickable| match *pickable {
        Pickable::SketchEntity { feature, entity } if feature == sketch => {
            let is_axis = match entity.reference() {
                Some(Reference::HorizontalAxis | Reference::VerticalAxis) => true,
                Some(Reference::Origin) => false,
                None => matches!(definition.entity(entity), Some(Entity::Line { .. })),
            };
            is_axis.then_some(RevolveAxis::Sketch(entity))
        }
        _ => None,
    });
    let body = match body_selection::bodies_in(selection).as_slice() {
        [] => None,
        [body] => Some(*body),
        [_, _, ..] => return Err(SweepSourceError::SeveralBodies),
    };
    Ok(Some(SweepSource {
        sketch,
        axis,
        regions,
        body,
    }))
}

fn guessed_sketch(
    document: &Document,
    evaluation: &Evaluation,
    editing: &SketchEditing,
    guess: Guess,
) -> Option<FeatureId> {
    let opened = editing
        .solid()
        .and_then(|solid| document.feature(solid))
        .and_then(|feature| feature.kind.solid())
        .map(SolidFeature::sketch);
    let last = || {
        document
            .active_features()
            .rev()
            .filter(|feature| feature.kind.sketch().is_some())
            .map(|feature| feature.id())
            .find(|sketch| visibility::is_shown(document, *sketch))
    };
    let unused = |sketch: &FeatureId| match guess {
        Guess::Never => false,
        Guess::Sweep => has_unswept_regions(document, evaluation, *sketch),
        Guess::Drill => !is_drilled(document, *sketch),
    };
    opened.or_else(last).filter(unused)
}

fn is_drilled(document: &Document, sketch: FeatureId) -> bool {
    document
        .active_features()
        .any(|feature| matches!(&feature.kind, FeatureKind::Hole(hole) if hole.sketch == sketch))
}

pub fn has_unswept_regions(
    document: &Document,
    evaluation: &Evaluation,
    sketch: FeatureId,
) -> bool {
    let users: Vec<&Feature> = document
        .active_features()
        .filter(|feature| match &feature.kind {
            FeatureKind::Solid(solid) => solid.sketch() == sketch,
            FeatureKind::Hole(hole) => hole.sketch == sketch,
            _ => false,
        })
        .collect();
    let regions = evaluation
        .feature(sketch)
        .and_then(|evaluated| evaluated.result.as_deref())
        .and_then(FeatureResult::sketch)
        .and_then(|result| result.regions());
    let regions = match regions {
        None => return users.is_empty(),
        Some(Err(_)) => return false,
        Some(Ok(regions)) if regions.is_empty() => {
            return users.is_empty() && is_open_profile(document, sketch);
        }
        Some(Ok(regions)) => regions,
    };
    swept_regions(document, sketch, regions).is_some_and(|swept| {
        regions
            .iter()
            .filter(|region| region.even_depth)
            .any(|region| !swept.contains(&region.region.key()))
    })
}

fn swept_regions(
    document: &Document,
    sketch: FeatureId,
    regions: &[SketchRegion],
) -> Option<BTreeSet<RegionKey>> {
    let mut swept = BTreeSet::new();
    for feature in document.active_features() {
        match &feature.kind {
            FeatureKind::Solid(solid) if solid.sketch() == sketch => {
                swept.extend(scene::chosen_regions(solid.regions(), regions));
            }
            FeatureKind::Hole(hole) if hole.sketch == sketch => return None,
            _ => {}
        }
    }
    Some(swept)
}

fn regions_left(document: &Document, evaluation: &Evaluation, sketch: FeatureId) -> RegionChoice {
    let Some(regions) = selection::sketch_regions(evaluation, sketch) else {
        return RegionChoice::All;
    };
    match swept_regions(document, sketch, regions) {
        Some(swept) if !swept.is_empty() => {
            let left: BTreeSet<RegionKey> = regions
                .iter()
                .filter(|region| region.even_depth)
                .map(|region| region.region.key())
                .filter(|key| !swept.contains(key))
                .collect();
            RegionChoice::Chosen(scene::region_references(&left, regions))
        }
        Some(_) | None => RegionChoice::All,
    }
}

pub fn may_guess_sketch(sweep: Sweep, selection: &Selection, lone_axis: bool) -> bool {
    selection.is_empty() || (sweep == Sweep::Revolve && lone_axis && selection.len() == 1)
}

#[derive(Debug, Clone, PartialEq)]
pub struct FaceProfile {
    pub body: FeatureId,
    pub faces: Vec<FaceKey>,
    pub axis: Option<AxisReference>,
}

impl FaceProfile {
    fn first(&self) -> Option<FaceChoice> {
        self.faces.first().map(|face| FaceChoice {
            body: self.body,
            face: *face,
        })
    }
}

pub fn faces_to_sweep(
    model: &Model,
    selection: &Selection,
    editing: &SketchEditing,
    sweep: Sweep,
    axes: &[(Pickable, AxisReference)],
) -> Option<Result<FaceProfile, FaceSweepError>> {
    let sketch_chosen = selection.iter().any(|pickable| {
        matches!(
            pickable,
            Pickable::SketchEntity { .. } | Pickable::SketchRegion { .. }
        )
    });
    if editing.feature().is_some() || sketch_chosen {
        return None;
    }
    let picked = selection.in_pick_order();
    let axis = match sweep {
        Sweep::Extrude => None,
        Sweep::Revolve => picked.iter().rev().find_map(|pickable| {
            axes.iter()
                .find(|(candidate, _)| candidate == pickable)
                .filter(|_| {
                    FaceChoice::of(*pickable)
                        .is_none_or(|face| !sketch_placement::is_flat(model, face))
                })
        }),
    };
    let faces: Vec<FaceChoice> = picked
        .iter()
        .filter(|pickable| axis.is_none_or(|(candidate, _)| candidate != *pickable))
        .filter_map(|pickable| FaceChoice::of(*pickable))
        .collect();
    if faces.is_empty() {
        return None;
    }
    Some(face_profile(
        model,
        &faces,
        axis.map(|(_, axis)| axis.clone()),
        sweep,
    ))
}

fn face_profile(
    model: &Model,
    faces: &[FaceChoice],
    axis: Option<AxisReference>,
    sweep: Sweep,
) -> Result<FaceProfile, FaceSweepError> {
    let body = faces
        .first()
        .map(|face| face.body)
        .ok_or(FaceSweepError::NoOutline)?;
    if faces.iter().any(|face| face.body != body) {
        return Err(FaceSweepError::SeveralBodies);
    }
    let shown = bodies::shown(model.evaluation(), body).ok_or(FaceSweepError::NoOutline)?;
    let planes = faces
        .iter()
        .map(|face| {
            bodies::find_face(shown, face.face)
                .and_then(|id| face_plane(&shown.solid, id))
                .ok_or(FaceSweepError::NotFlat { sweep })
        })
        .collect::<Result<Vec<Plane>, _>>()?;
    if let Some((first, rest)) = planes.split_first()
        && !rest.iter().all(|plane| same_plane(first, plane))
    {
        return Err(FaceSweepError::NotInOnePlane);
    }
    if sweep == Sweep::Revolve && axis.is_none() {
        return Err(FaceSweepError::NoAxis);
    }
    Ok(FaceProfile {
        body,
        faces: faces.iter().map(|face| face.face).collect(),
        axis,
    })
}

pub fn same_plane(first: &Plane, second: &Plane) -> bool {
    first.normal().cross(second.normal()).length() <= ANGULAR_RESOLUTION
        && first.normal().dot(second.normal()) > 0.0
        && first.signed_distance(second.origin()).abs() <= LINEAR_RESOLUTION
}

pub fn create_on_faces(
    model: &Model,
    sweep: Sweep,
    profile: &FaceProfile,
) -> Result<(Transaction, FeatureId, String), FaceSweepError> {
    let document = model.document();
    let first = profile.first().ok_or(FaceSweepError::NoOutline)?;
    let (attachment, plane) = sketch_placement::attachment_at(model, first, document.bar_index())
        .map_err(|error| FaceSweepError::Attachment(error.into()))?;
    let shown = bodies::shown(model.evaluation(), profile.body).ok_or(FaceSweepError::NoOutline)?;
    let ids = profile
        .faces
        .iter()
        .map(|face| bodies::find_face(shown, *face).ok_or(FaceSweepError::NoOutline))
        .collect::<Result<Vec<_>, _>>()?;
    let outline = projecting::face_projections(shown, profile.body, &ids, &plane);
    if outline.is_empty() {
        return Err(FaceSweepError::NoOutline);
    }
    let sketch_name = editing::next_sketch_name(document);
    let name = editing::next_feature_name(document, sweep.label());
    let mut transaction = document.transaction(format!("Create {name}"));
    let sketch = transaction.add_feature(
        sketch_name.clone(),
        FeatureKind::Sketch(SketchFeature::on_face(Sketch::new(plane), attachment)),
    );
    transaction.edit(Edit::SetFeatureHidden {
        id: sketch,
        hidden: true,
    });
    for (source, edges) in &outline {
        transaction.add_projection(sketch, source.clone(), edges);
    }
    let operation = BodyOperation::Add(profile.body);
    let starts = Starts::of(model);
    let solid = match (sweep, &profile.axis) {
        (Sweep::Extrude, _) => SolidFeature::Extrude(Extrude {
            sketch,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(
                starts.length(Remembered::ExtrudeDistance, DEFAULT_DISTANCE),
                false,
            ),
            operation,
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
        }),
        (Sweep::Revolve, Some(axis)) => SolidFeature::Revolve(Revolve {
            sketch,
            regions: RegionChoice::All,
            axis: RevolveAxis::Model(axis.clone()),
            extent: starting_turn(&starts),
            operation,
            start: None,
            other_bodies: Vec::new(),
            side: None,
            wall: None,
        }),
        (Sweep::Revolve, None) => return Err(FaceSweepError::NoAxis),
    };
    let feature = transaction.add_feature(name.clone(), FeatureKind::Solid(solid));
    let faces = match profile.faces.len() {
        1 => "face",
        _ => "faces",
    };
    let action = match sweep {
        Sweep::Extrude => "extrudes",
        Sweep::Revolve => "revolves",
    };
    let told = format!(
        "{name} {action} the selected {faces} out of its body, following the edges through \
         {sketch_name}. Edit {sketch_name} to change the outline."
    );
    Ok((transaction.finish(), feature, told))
}

pub fn create_on_faces_actions(model: &Model, sweep: Sweep, profile: &FaceProfile) -> Vec<Action> {
    match create_on_faces(model, sweep, profile) {
        Ok((transaction, feature, told)) => vec![
            Action::Apply(transaction),
            Action::Editing(EditingCommand::OpenSolid(feature)),
            Action::Inform(Notice::info(told)),
        ],
        Err(reason) => vec![Action::Inform(Notice::warning(format!(
            "{}: {reason}.",
            sweep.label()
        )))],
    }
}

pub fn with_model_axis(
    source: SweepSource,
    selection: &Selection,
    axes: &[(Pickable, AxisReference)],
) -> SweepSource {
    let sketch_line = |pickable: Pickable| match (pickable, &source.axis) {
        (Pickable::SketchEntity { feature, entity }, Some(RevolveAxis::Sketch(line))) => {
            feature == source.sketch && entity == *line
        }
        _ => false,
    };
    let latest = selection
        .in_pick_order()
        .into_iter()
        .rev()
        .find_map(|pickable| {
            if sketch_line(pickable) {
                return source.axis.clone();
            }
            let in_sketch = matches!(
                pickable,
                Pickable::SketchEntity { feature, .. } if feature == source.sketch
            );
            axes.iter()
                .find(|(candidate, _)| *candidate == pickable && !in_sketch)
                .map(|(_, axis)| RevolveAxis::Model(axis.clone()))
        });
    SweepSource {
        axis: latest.or(source.axis),
        ..source
    }
}

pub fn with_selected_outline(
    model: &Model,
    sweep: Sweep,
    source: SweepSource,
    selection: &Selection,
) -> SweepSource {
    let Some(regions) = selection::sketch_regions(model.evaluation(), source.sketch) else {
        return source;
    };
    let mut chosen = selected_regions(selection, source.sketch);
    chosen.extend(enclosed_regions(model, sweep, &source, selection, regions));
    if chosen.is_empty() {
        return source;
    }
    SweepSource {
        regions: RegionChoice::Chosen(scene::region_references(&chosen, regions)),
        ..source
    }
}

pub fn selected_regions(selection: &Selection, sketch: FeatureId) -> BTreeSet<RegionKey> {
    selection
        .iter()
        .filter_map(|pickable| match pickable {
            Pickable::SketchRegion { feature, region } if feature == sketch => Some(region),
            _ => None,
        })
        .collect()
}

fn enclosed_regions(
    model: &Model,
    sweep: Sweep,
    source: &SweepSource,
    selection: &Selection,
    regions: &[SketchRegion],
) -> BTreeSet<RegionKey> {
    let Some(feature) = model.document().feature(source.sketch) else {
        return BTreeSet::new();
    };
    let Some(displayed) = model.displayed_sketch(feature) else {
        return BTreeSet::new();
    };
    let axis = match (sweep, &source.axis) {
        (Sweep::Revolve, Some(RevolveAxis::Sketch(axis))) => Some(*axis),
        _ => None,
    };
    let outline: Vec<Vec<Point2>> = selection
        .iter()
        .filter_map(|pickable| match pickable {
            Pickable::SketchEntity { feature, entity }
                if feature == source.sketch && Some(entity) != axis =>
            {
                displayed.polyline(entity, OUTLINE_SEGMENT_ANGLE)
            }
            _ => None,
        })
        .collect();
    if outline.is_empty() || !closed(&outline) {
        return BTreeSet::new();
    }
    regions
        .iter()
        .filter(|region| {
            region
                .reference()
                .anchor()
                .is_some_and(|anchor| encloses(&outline, anchor))
        })
        .map(|region| region.region.key())
        .collect()
}

fn closed(outline: &[Vec<Point2>]) -> bool {
    let ends: Vec<Point2> = outline
        .iter()
        .filter_map(|polyline| Some((*polyline.first()?, *polyline.last()?)))
        .filter(|(first, last)| first.distance(*last) > OUTLINE_GAP)
        .flat_map(|(first, last)| [first, last])
        .collect();
    ends.iter().enumerate().all(|(index, end)| {
        ends.iter()
            .enumerate()
            .any(|(other, point)| other != index && point.distance(*end) <= OUTLINE_GAP)
    })
}

fn encloses(outline: &[Vec<Point2>], point: Point2) -> bool {
    let crossings = outline
        .iter()
        .flat_map(|polyline| polyline.windows(2))
        .filter(|pair| match pair {
            [start, end] => {
                (start.y > point.y) != (end.y > point.y)
                    && point.x
                        < start.x + (point.y - start.y) / (end.y - start.y) * (end.x - start.x)
            }
            _ => false,
        })
        .count();
    crossings % 2 == 1
}

pub fn axis_name(document: &Document, sketch: FeatureId, axis: &RevolveAxis) -> String {
    match axis {
        RevolveAxis::Sketch(line) => editing::edited_sketch(document, sketch).map_or_else(
            || "the chosen line".to_owned(),
            |sketch| sketch.entity_label(*line),
        ),
        RevolveAxis::Model(axis) => describe_axis(document, axis),
    }
}

pub fn face_body(document: &Document, sketch: FeatureId) -> Option<FeatureId> {
    let body = match document.feature(sketch)?.kind.attachment()? {
        SketchAttachment::Face(face) => face.body,
        SketchAttachment::Datum(_) | SketchAttachment::Frame { .. } => return None,
    };
    document
        .bodies_standing()
        .contains(&body.maker())
        .then_some(body)
}

pub fn degrees(value: f64) -> Expression {
    Expression::measure(value, Unit::Degree)
}

pub fn default_wall(starts: &Starts) -> Wall {
    Wall {
        thickness: starts.length(Remembered::WallThickness, DEFAULT_WALL_THICKNESS),
        side: WallSide::Centred,
    }
}

pub fn starting_turn(starts: &Starts) -> RevolveExtent {
    starts
        .remembered(Remembered::RevolveAngle)
        .map_or(RevolveExtent::Full, |angle| RevolveExtent::OneSide {
            angle,
            reversed: false,
        })
}

fn is_open_profile(document: &Document, sketch: FeatureId) -> bool {
    let Some(sketch) = editing::edited_sketch(document, sketch) else {
        return false;
    };
    let has_curves = sketch
        .entities()
        .any(|(id, _)| !sketch.is_construction(id) && profile_curve(sketch, id).is_some());
    has_curves && sketch_regions(sketch).is_ok_and(|regions| regions.is_empty())
}

fn open_profile_wall(document: &Document, sketch: FeatureId, starts: &Starts) -> Option<Box<Wall>> {
    is_open_profile(document, sketch).then(|| Box::new(default_wall(starts)))
}

pub fn create(
    document: &Document,
    reach: Reach<'_>,
    sweep: Sweep,
    source: SweepSource,
    starts: &Starts,
) -> (Transaction, FeatureId, Option<String>) {
    let name = editing::next_feature_name(document, sweep.label());
    let wall = match source.regions {
        RegionChoice::All => open_profile_wall(document, source.sketch, starts),
        RegionChoice::Chosen(_) => None,
    };
    let standing_on = match sweep {
        Sweep::Extrude => face_body(document, source.sketch),
        Sweep::Revolve => None,
    };
    let mut told = None;
    let solid = match sweep {
        Sweep::Extrude => {
            let template = Extrude {
                sketch: source.sketch,
                regions: source.regions.clone(),
                extent: ExtrudeExtent::one_side(
                    starts.length(Remembered::ExtrudeDistance, DEFAULT_DISTANCE),
                    false,
                ),
                operation: BodyOperation::NewBody,
                start: None,
                other_bodies: Vec::new(),
                taper: None,
                wall,
                direction: None,
            };
            let detected = extrusion_detection::extrusion(
                document,
                reach,
                &template,
                standing_on,
                source.body,
            );
            told = detected.told;
            SolidFeature::Extrude(Extrude {
                extent: detected.extent,
                operation: detected.operation,
                ..template
            })
        }
        Sweep::Revolve => {
            let solid = SolidFeature::Revolve(Revolve {
                sketch: source.sketch,
                regions: source.regions.clone(),
                axis: source
                    .axis
                    .unwrap_or(RevolveAxis::Sketch(EntityId::VERTICAL_AXIS)),
                extent: starting_turn(starts),
                operation: BodyOperation::NewBody,
                start: None,
                other_bodies: Vec::new(),
                side: None,
                wall,
            });
            let operation = match source.body {
                Some(selected) => BodyOperation::Add(selected),
                None => default_operation(document, reach, &solid),
            };
            with_operation(&solid, operation)
        }
    };
    let mut transaction = document.transaction(format!("Create {name}"));
    let feature = transaction.add_feature(name, FeatureKind::Solid(solid));
    if starts.pieces() == Pieces::BodyEach {
        transaction.edit(Edit::SetFeaturePieces {
            id: feature,
            pieces: Pieces::BodyEach,
        });
    }
    if visibility::is_shown(document, source.sketch) {
        transaction.edit(Edit::SetFeatureHidden {
            id: source.sketch,
            hidden: true,
        });
    }
    (transaction.finish(), feature, told)
}

#[derive(Debug, Clone, Copy)]
pub struct Reach<'a> {
    pub evaluation: &'a Evaluation,
    pub parameters: &'a ParameterValues,
}

impl<'a> Reach<'a> {
    pub fn of(model: &'a Model) -> Self {
        Self {
            evaluation: model.evaluation(),
            parameters: model.parameters(),
        }
    }
}

pub fn default_operation(
    document: &Document,
    reach: Reach<'_>,
    solid: &SolidFeature,
) -> BodyOperation {
    body_reach::reached_body(document, reach.evaluation, reach.parameters, solid)
        .map_or(BodyOperation::NewBody, BodyOperation::Add)
}

pub fn create_actions(
    model: &Model,
    sweep: Sweep,
    source: SweepSource,
    starts: &Starts,
) -> Vec<Action> {
    let (transaction, feature, told) =
        create(model.document(), Reach::of(model), sweep, source, starts);
    let mut actions = vec![
        Action::Apply(transaction),
        Action::Editing(EditingCommand::OpenSolid(feature)),
    ];
    actions.extend(told.map(|words| Action::Inform(Notice::info(words))));
    actions
}

pub fn edit(document: &Document, feature: FeatureId, solid: SolidFeature) -> Option<Transaction> {
    let name = &document.feature(feature)?.name;
    let fields = solid_panel::captioned(&solid);
    let transaction = Transaction::single(
        format!("Edit {name}"),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Solid(solid.clone()),
        },
    );
    Some(document.carrying_owners(transaction, feature, fields))
}

pub fn with_regions(solid: &SolidFeature, regions: RegionChoice) -> SolidFeature {
    let mut changed = solid.clone();
    match &mut changed {
        SolidFeature::Extrude(extrude) => extrude.regions = regions,
        SolidFeature::Revolve(revolve) => revolve.regions = regions,
        SolidFeature::Sweep(sweep) => sweep.regions = regions,
    }
    changed
}

pub fn with_start(solid: &SolidFeature, start: Option<SolidStart>) -> SolidFeature {
    let mut changed = solid.clone();
    match &mut changed {
        SolidFeature::Extrude(extrude) => extrude.start = start,
        SolidFeature::Revolve(revolve) => revolve.start = start,
        SolidFeature::Sweep(_) => {}
    }
    changed
}

pub fn with_wall(solid: &SolidFeature, wall: Option<Wall>) -> SolidFeature {
    let wall = wall.map(Box::new);
    let mut changed = solid.clone();
    match &mut changed {
        SolidFeature::Extrude(extrude) => extrude.wall = wall,
        SolidFeature::Revolve(revolve) => {
            if wall.is_some() {
                revolve.side = None;
            }
            revolve.wall = wall;
        }
        SolidFeature::Sweep(_) => {}
    }
    changed
}

pub fn with_operation(solid: &SolidFeature, operation: BodyOperation) -> SolidFeature {
    let mut changed = solid.clone();
    match &mut changed {
        SolidFeature::Extrude(extrude) => extrude.operation = operation,
        SolidFeature::Revolve(revolve) => revolve.operation = operation,
        SolidFeature::Sweep(sweep) => sweep.operation = operation,
    }
    let target = operation.target();
    let others = changed.other_bodies_mut();
    match operation {
        BodyOperation::Remove(_) => others.retain(|other| Some(*other) != target),
        BodyOperation::NewBody | BodyOperation::Add(_) | BodyOperation::Intersect(_) => {
            others.clear();
        }
    }
    changed
}

pub fn with_other_bodies(solid: &SolidFeature, other_bodies: Vec<FeatureId>) -> SolidFeature {
    let mut changed = solid.clone();
    *changed.other_bodies_mut() = other_bodies;
    changed
}

pub fn toggle_region(model: &Model, feature: FeatureId, region: RegionKey) -> Option<Transaction> {
    let document = model.document();
    let owner = document.feature(feature)?;
    let solid = owner.kind.solid()?;
    let (_, regions) = selection::swept_regions(document, model.evaluation(), feature)?;
    let mut chosen = scene::chosen_regions(solid.regions(), regions);
    if !chosen.remove(&region) {
        chosen.insert(region);
    }
    let choice = RegionChoice::Chosen(scene::region_references(&chosen, regions));
    Some(Transaction::single(
        format!("Choose regions of {}", owner.name),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Solid(with_regions(solid, choice)),
        },
    ))
}

pub fn clear_regions(model: &Model, feature: &Feature) -> Result<Transaction, RegionError> {
    let Some(solid) = feature.kind.solid() else {
        return Err(RegionError::NotASweep {
            name: feature.name.clone(),
        });
    };
    let id = feature.id();
    let nothing_chosen = selection::swept_regions(model.document(), model.evaluation(), id)
        .is_some_and(|(_, regions)| scene::chosen_regions(solid.regions(), regions).is_empty());
    if nothing_chosen {
        return Err(RegionError::NoneChosen {
            name: feature.name.clone(),
        });
    }
    Ok(Transaction::single(
        format!("Clear the chosen regions of {}", feature.name),
        Edit::SetFeatureKind {
            id,
            kind: FeatureKind::Solid(with_regions(solid, RegionChoice::Chosen(Vec::new()))),
        },
    ))
}

pub fn pieces_change(
    model: &Model,
    feature: &Feature,
    pieces: Pieces,
) -> Result<Transaction, CheckError> {
    let label = match pieces {
        Pieces::OneBody => format!("Make one body of {}", feature.name),
        Pieces::BodyEach => format!("Make a body of each piece of {}", feature.name),
    };
    field::checked(
        model.document(),
        Transaction::single(
            label,
            Edit::SetFeaturePieces {
                id: feature.id(),
                pieces,
            },
        ),
    )
}

pub fn select_all_regions(model: &Model, feature: &Feature) -> Result<Transaction, RegionError> {
    let Some(solid) = feature.kind.solid() else {
        return Err(RegionError::TakesNoRegions {
            name: feature.name.clone(),
        });
    };
    if solid.wall().is_some() {
        return Err(RegionError::ThinWall {
            name: feature.name.clone(),
        });
    }
    if matches!(solid.regions(), RegionChoice::All) {
        return Err(RegionError::AlreadyEvery {
            name: feature.name.clone(),
        });
    }
    let sketch = model
        .document()
        .feature(solid.sketch())
        .map_or("the sketch", |sketch| sketch.name.as_str());
    Ok(Transaction::single(
        format!("Select all regions of {sketch} for {}", feature.name),
        Edit::SetFeatureKind {
            id: feature.id(),
            kind: FeatureKind::Solid(with_regions(solid, RegionChoice::All)),
        },
    ))
}

pub fn take_sketch_regions(
    model: &Model,
    feature: &Feature,
    sketch: FeatureId,
) -> Result<Transaction, RegionError> {
    let Some(solid) = feature.kind.solid() else {
        return Err(RegionError::TakesNoRegions {
            name: feature.name.clone(),
        });
    };
    if solid.sketch() == sketch {
        return select_all_regions(model, feature);
    }
    let document = model.document();
    let name = document
        .feature(sketch)
        .filter(|candidate| candidate.kind.sketch().is_some())
        .map(|sketch| sketch.name.clone())
        .ok_or(RegionError::NotASketch { feature: sketch })?;
    let transaction = Transaction::single(
        format!("Take all regions of {name} for {}", feature.name),
        Edit::SetFeatureKind {
            id: feature.id(),
            kind: FeatureKind::Solid(solid_panel::with_sketch(solid, sketch)),
        },
    );
    field::checked(document, transaction)
}

#[cfg(test)]
mod tests {
    use caditor_document::{CancelToken, ModelEvaluator, Recompute};
    use caditor_geometry::{Plane, Point2, Point3, Vector3};
    use caditor_sketch::Sketch;

    use super::*;
    use crate::units::LengthUnit;

    fn document_with_sketches() -> (Document, FeatureId, FeatureId, EntityId) {
        let mut document = Document::default();
        let mut first = Sketch::new(Plane::XY);
        first.add_line(Point2::ZERO, Point2::new(10.0, 0.0));
        let mut second = Sketch::new(Plane::XZ);
        let line = second.add_line(Point2::ZERO, Point2::new(0.0, 10.0));
        let mut transaction = document.transaction("Sketches");
        let base = transaction.add_feature("Base", FeatureKind::from(first));
        let side = transaction.add_feature("Side", FeatureKind::from(second));
        document.apply(transaction.finish()).unwrap();
        (document, base, side, line)
    }

    fn source(
        document: &Document,
        selection: &Selection,
        editing: &SketchEditing,
        may_guess: bool,
    ) -> Option<SweepSource> {
        sweep_source(
            document,
            &Evaluation::default(),
            selection,
            editing,
            Guess::sweep_when(may_guess),
        )
        .unwrap()
    }

    #[test]
    fn the_source_prefers_the_edited_sketch_then_the_selection_then_the_last_sketch() {
        let (document, base, side, line) = document_with_sketches();
        let mut selection = Selection::default();
        let idle = SketchEditing::default();

        assert_eq!(
            source(&document, &selection, &idle, true),
            Some(SweepSource {
                sketch: side,
                axis: None,
                regions: RegionChoice::All,
                body: None,
            })
        );
        assert_eq!(
            source(&document, &selection, &SketchEditing::editing(base), false).map(|s| s.sketch),
            Some(base)
        );
        selection.replace_with(Pickable::SketchEntity {
            feature: side,
            entity: line,
        });
        assert_eq!(
            source(&document, &selection, &SketchEditing::editing(side), false),
            Some(SweepSource {
                sketch: side,
                axis: Some(RevolveAxis::Sketch(line)),
                regions: RegionChoice::All,
                body: None,
            })
        );
        assert_eq!(
            source(&Document::default(), &Selection::default(), &idle, true),
            None
        );
    }

    #[test]
    fn no_sketch_is_guessed_for_a_selection_that_holds_none() {
        let (document, _, side, _) = document_with_sketches();
        let idle = SketchEditing::default();
        let mut selection = Selection::default();
        selection.replace_with(Pickable::Axis(crate::selection::Axis::X));

        assert_eq!(source(&document, &selection, &idle, false), None);
        assert_eq!(
            source(&document, &selection, &idle, true).map(|source| source.sketch),
            Some(side)
        );
    }

    #[test]
    fn curves_of_two_sketches_are_refused_and_the_last_picked_line_is_the_axis() {
        let (mut document, base, side, line) = document_with_sketches();
        let mut lines = Sketch::new(Plane::XZ);
        let first = lines.add_line(Point2::ZERO, Point2::new(0.0, 10.0));
        let second = lines.add_line(Point2::new(5.0, 0.0), Point2::new(5.0, 10.0));
        let mut transaction = document.transaction("Lines");
        let both = transaction.add_feature("Lines", FeatureKind::from(lines));
        document.apply(transaction.finish()).unwrap();
        let idle = SketchEditing::default();
        let evaluation = Evaluation::default();
        let mut selection = Selection::default();
        selection.toggle(Pickable::SketchEntity {
            feature: side,
            entity: line,
        });
        selection.toggle(Pickable::SketchEntity {
            feature: base,
            entity: EntityId::HORIZONTAL_AXIS,
        });

        assert_eq!(
            sweep_source(&document, &evaluation, &selection, &idle, Guess::Never),
            Err(SweepSourceError::SeveralSketches)
        );

        selection.clear();
        selection.toggle(Pickable::SketchEntity {
            feature: both,
            entity: second,
        });
        selection.toggle(Pickable::SketchEntity {
            feature: both,
            entity: first,
        });

        assert_eq!(
            source(&document, &selection, &idle, false).and_then(|source| source.axis),
            Some(RevolveAxis::Sketch(first))
        );
    }

    fn rectangle(plane: Plane, min: Point2, max: Point2) -> Sketch {
        let mut sketch = Sketch::new(plane);
        let corners = [
            min,
            Point2::new(max.x, min.y),
            max,
            Point2::new(min.x, max.y),
        ];
        for (index, corner) in corners.iter().enumerate() {
            sketch.add_line(*corner, corners[(index + 1) % corners.len()]);
        }
        sketch
    }

    fn sketched(document: &mut Document, name: &str, sketch: Sketch) -> FeatureId {
        let mut transaction = document.transaction(name);
        let feature = transaction.add_feature(name, FeatureKind::from(sketch));
        document.apply(transaction.finish()).unwrap();
        feature
    }

    fn evaluated(document: &Document) -> Evaluation {
        Recompute::default().run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
    }

    fn swept(document: &mut Document, sweep: Sweep, sketch: FeatureId) -> FeatureId {
        swept_from(document, sweep, sketch, None)
    }

    fn swept_from(
        document: &mut Document,
        sweep: Sweep,
        sketch: FeatureId,
        axis: Option<RevolveAxis>,
    ) -> FeatureId {
        let evaluation = evaluated(document);
        let parameters = ParameterValues::evaluate(document);
        let reach = Reach {
            evaluation: &evaluation,
            parameters: &parameters,
        };
        let source = SweepSource {
            sketch,
            axis,
            regions: RegionChoice::All,
            body: None,
        };
        let (transaction, feature, _) = create(
            document,
            reach,
            sweep,
            source,
            &Starts::defaults(LengthUnit::Millimetre),
        );
        document.apply(transaction).unwrap();
        feature
    }

    fn operation(document: &Document, feature: FeatureId) -> BodyOperation {
        document
            .feature(feature)
            .and_then(|feature| feature.kind.solid())
            .map(SolidFeature::operation)
            .unwrap()
    }

    fn block(document: &mut Document, name: &str, min: Point2, max: Point2) -> FeatureId {
        let sketch = sketched(document, name, rectangle(Plane::XY, min, max));
        swept(document, Sweep::Extrude, sketch)
    }

    #[test]
    fn a_new_extrusion_away_from_every_body_makes_a_new_body() {
        let mut document = Document::default();

        let first = block(&mut document, "Base", Point2::ZERO, Point2::new(20.0, 20.0));
        let second = block(
            &mut document,
            "Apart",
            Point2::new(50.0, 0.0),
            Point2::new(60.0, 10.0),
        );

        assert_eq!(document.feature(first).unwrap().name, "Extrude 1");
        assert_eq!(operation(&document, first), BodyOperation::NewBody);
        assert_eq!(operation(&document, second), BodyOperation::NewBody);
        assert_eq!(document.bodies_standing(), vec![first, second]);
    }

    #[test]
    fn a_new_extrusion_overlapping_or_touching_a_body_adds_to_it() {
        let mut document = Document::default();
        let base = block(&mut document, "Base", Point2::ZERO, Point2::new(20.0, 20.0));

        let overlapping = block(
            &mut document,
            "Overlapping",
            Point2::new(10.0, 10.0),
            Point2::new(30.0, 30.0),
        );
        let beside = block(
            &mut document,
            "Beside",
            Point2::new(-10.0, 0.0),
            Point2::ZERO.with_y(10.0),
        );
        let on_top = sketched(
            &mut document,
            "On top",
            rectangle(
                Plane::from_frame(Point3::new(0.0, 0.0, 10.0), Vector3::Z, Vector3::X).unwrap(),
                Point2::new(2.0, 2.0),
                Point2::new(8.0, 8.0),
            ),
        );
        let standing_on_top = swept(&mut document, Sweep::Extrude, on_top);

        assert_eq!(operation(&document, overlapping), BodyOperation::Add(base));
        assert_eq!(operation(&document, beside), BodyOperation::Add(base));
        assert_eq!(
            operation(&document, standing_on_top),
            BodyOperation::Add(base)
        );
        assert_eq!(document.bodies_standing(), vec![base]);
    }

    #[test]
    fn a_new_extrusion_reaching_two_bodies_adds_to_the_one_it_overlaps_most() {
        let reaching_both = |from: f64, to: f64| {
            let mut document = Document::default();
            let left = block(&mut document, "Left", Point2::ZERO, Point2::new(20.0, 20.0));
            let right = block(
                &mut document,
                "Right",
                Point2::new(30.0, 0.0),
                Point2::new(50.0, 20.0),
            );
            let across = block(
                &mut document,
                "Across",
                Point2::new(from, 0.0),
                Point2::new(to, 20.0),
            );
            (
                operation(&document, right),
                operation(&document, across),
                left,
                right,
            )
        };

        let (right_made, mostly_right, _, right) = reaching_both(15.0, 45.0);
        let (_, mostly_left, left, _) = reaching_both(5.0, 35.0);

        assert_eq!(right_made, BodyOperation::NewBody);
        assert_eq!(mostly_right, BodyOperation::Add(right));
        assert_eq!(mostly_left, BodyOperation::Add(left));
    }

    fn extruded_toward(
        document: &mut Document,
        sketch: FeatureId,
        body: Option<FeatureId>,
    ) -> FeatureId {
        let evaluation = evaluated(document);
        let parameters = ParameterValues::evaluate(document);
        let reach = Reach {
            evaluation: &evaluation,
            parameters: &parameters,
        };
        let source = SweepSource {
            sketch,
            axis: None,
            regions: RegionChoice::All,
            body,
        };
        let (transaction, feature, _) = create(
            document,
            reach,
            Sweep::Extrude,
            source,
            &Starts::defaults(LengthUnit::Millimetre),
        );
        document.apply(transaction).unwrap();
        feature
    }

    fn is_reversed(document: &Document, feature: FeatureId) -> bool {
        match document
            .feature(feature)
            .and_then(|feature| feature.kind.solid())
        {
            Some(SolidFeature::Extrude(extrude)) => matches!(
                extrude.extent,
                ExtrudeExtent::OneSide { reversed: true, .. }
            ),
            _ => panic!("expected an extrusion"),
        }
    }

    fn at_height(document: &mut Document, name: &str, height: f64) -> FeatureId {
        sketched(
            document,
            name,
            rectangle(
                Plane::from_frame(Point3::new(0.0, 0.0, height), Vector3::Z, Vector3::X).unwrap(),
                Point2::new(2.0, 2.0),
                Point2::new(8.0, 8.0),
            ),
        )
    }

    #[test]
    fn an_extrusion_pointing_away_from_a_body_it_would_reach_the_other_way_turns_toward_it() {
        let mut document = Document::default();
        let base = block(&mut document, "Base", Point2::ZERO, Point2::new(20.0, 20.0));
        let above = at_height(&mut document, "Above", 15.0);
        let far = at_height(&mut document, "Far", 40.0);

        let turned = extruded_toward(&mut document, above, None);
        let apart = extruded_toward(&mut document, far, None);

        let mut other = Document::default();
        let target = block(&mut other, "Base", Point2::ZERO, Point2::new(20.0, 20.0));
        let chosen = at_height(&mut other, "Chosen", 15.0);
        let asked_for = extruded_toward(&mut other, chosen, Some(target));

        assert_eq!(operation(&document, turned), BodyOperation::Add(base));
        assert!(is_reversed(&document, turned));
        assert_eq!(operation(&document, apart), BodyOperation::NewBody);
        assert!(!is_reversed(&document, apart));
        assert_eq!(operation(&other, asked_for), BodyOperation::Add(target));
        assert!(is_reversed(&other, asked_for));
    }

    #[test]
    fn an_extrusion_reaching_a_body_forward_is_never_turned() {
        let mut document = Document::default();
        let base = block(&mut document, "Base", Point2::ZERO, Point2::new(20.0, 20.0));
        let on_top = at_height(&mut document, "On top", 10.0);
        let inside = at_height(&mut document, "Inside", 5.0);

        let stacked = extruded_toward(&mut document, on_top, None);
        let overlapping = extruded_toward(&mut document, inside, None);

        assert_eq!(operation(&document, stacked), BodyOperation::Add(base));
        assert!(!is_reversed(&document, stacked));
        assert_eq!(operation(&document, overlapping), BodyOperation::Add(base));
        assert!(!is_reversed(&document, overlapping));
    }

    #[test]
    fn a_new_revolve_adds_to_the_body_it_passes_through_and_otherwise_makes_a_new_one() {
        let mut document = Document::default();
        let base = block(&mut document, "Base", Point2::ZERO, Point2::new(20.0, 20.0));
        let ring = sketched(
            &mut document,
            "Ring",
            rectangle(Plane::XZ, Point2::new(5.0, 2.0), Point2::new(10.0, 5.0)),
        );
        let far = sketched(
            &mut document,
            "Far ring",
            rectangle(Plane::XZ, Point2::new(5.0, 40.0), Point2::new(10.0, 45.0)),
        );

        let through = swept(&mut document, Sweep::Revolve, ring);
        let apart = swept_from(
            &mut document,
            Sweep::Revolve,
            far,
            Some(RevolveAxis::Sketch(EntityId::VERTICAL_AXIS)),
        );

        assert_eq!(document.feature(through).unwrap().name, "Revolve 1");
        assert_eq!(operation(&document, through), BodyOperation::Add(base));
        assert_eq!(operation(&document, apart), BodyOperation::NewBody);
    }

    #[test]
    fn new_features_skip_a_body_a_combine_consumed() {
        let mut document = Document::default();
        let target = block(
            &mut document,
            "Block",
            Point2::ZERO,
            Point2::new(20.0, 20.0),
        );
        let tool = block(
            &mut document,
            "Peg",
            Point2::new(30.0, 0.0),
            Point2::new(40.0, 10.0),
        );
        let mut transaction = document.transaction("Two bodies joined");
        transaction.add_feature(
            "Combine 1",
            FeatureKind::Combine(caditor_document::Combine::new(
                target,
                tool,
                caditor_document::CombineOperation::Join,
            )),
        );
        document.apply(transaction.finish()).unwrap();

        let over_the_peg = block(
            &mut document,
            "Over the peg",
            Point2::new(32.0, 2.0),
            Point2::new(38.0, 8.0),
        );

        assert_eq!(operation(&document, tool), BodyOperation::NewBody);
        assert_eq!(document.bodies_standing(), vec![target]);
        assert_eq!(
            operation(&document, over_the_peg),
            BodyOperation::Add(target)
        );
    }
}
