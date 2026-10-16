use caditor_document::{Datum, FeatureId, FeatureKind, PatternKind, SolidFeature, Transaction};
use caditor_sketch::EntityId;
use egui::{Context, Id};

use crate::{
    coil_tools,
    datum_panel::{self, FramePart},
    datum_tools,
    editing::EditingCommand,
    helix_tools, hole_panel, hole_placement, hole_tools, image_tools, loft_tools,
    mate_tools::{self, MatePart},
    measurement_tools::{self, MeasuredPart},
    mirror_tools,
    model::{Action, Model, Notice},
    move_tools,
    pattern_tools::{self, Reference},
    pipe_tools, primitive_tools, scale_tools,
    selection::{Pickable, Selection},
    shell_tools, sketch_pattern_tools, solid_panel, split_face_tools, split_tools, sweep_tools,
    thread_tools,
};

pub const STOP_HINT: &str = "Esc: stop choosing";
const GONE: &str = "The feature no longer exists";
const AXIS: &str = "an axis, straight edge or round face";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    One,
    Forward,
    Backward,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Slot {
    RevolveAxis,
    ExtrudeTarget(Side),
    StartPlane,
    HoleTarget,
    HolePlace,
    RevolveTarget,
    ExtrudeDirection,
    MirrorPlane,
    SplitPlane,
    SplitDirection,
    PatternDirection,
    PatternSecond,
    PatternPath,
    PatternBase,
    DatumBase,
    DatumRotation,
    MoveAxis,
    PrimitivePlace,
    CoilAxis,
    HelixAxis,
    MateMoving(MatePart),
    MateTarget(MatePart),
    FrameOrigin,
    FrameAxis,
    FramePlane,
    ScaleCentre,
    ThreadFace,
    MeasuredItem(MeasuredPart),
    HoleEdge(EntityId),
    HoleConcentric(EntityId),
    HoleAdd,
    ShellOwnThickness,
    SweepPath,
    LoftProfiles,
    LoftEnd,
    ImagePlace,
    ImageCalibration,
}

impl Slot {
    pub fn keeps_choosing(self) -> bool {
        matches!(
            self,
            Self::SweepPath | Self::LoftProfiles | Self::ShellOwnThickness | Self::HoleAdd
        )
    }

    pub fn takes_principal_geometry(self) -> bool {
        match self {
            Self::RevolveAxis
            | Self::ExtrudeTarget(_)
            | Self::StartPlane
            | Self::HoleTarget
            | Self::RevolveTarget
            | Self::ExtrudeDirection
            | Self::MirrorPlane
            | Self::SplitPlane
            | Self::SplitDirection
            | Self::PatternDirection
            | Self::PatternSecond
            | Self::PatternBase
            | Self::DatumBase
            | Self::DatumRotation
            | Self::MoveAxis
            | Self::PrimitivePlace
            | Self::CoilAxis
            | Self::HelixAxis
            | Self::MateTarget(_)
            | Self::FrameOrigin
            | Self::FrameAxis
            | Self::FramePlane
            | Self::ScaleCentre
            | Self::MeasuredItem(_)
            | Self::HoleEdge(_)
            | Self::HoleConcentric(_)
            | Self::LoftEnd
            | Self::ImagePlace => true,
            Self::HolePlace
            | Self::ImageCalibration
            | Self::HoleAdd
            | Self::ShellOwnThickness
            | Self::ThreadFace
            | Self::SweepPath
            | Self::LoftProfiles
            | Self::PatternPath
            | Self::MateMoving(_) => false,
        }
    }
}

pub const MAX_HELD: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Picking {
    pub feature: FeatureId,
    pub slot: Slot,
    pub pending: [Option<Pickable>; MAX_HELD],
}

impl Picking {
    pub fn new(feature: FeatureId, slot: Slot) -> Self {
        Self {
            feature,
            slot,
            pending: [None; MAX_HELD],
        }
    }

    pub fn is_for(&self, feature: FeatureId, slot: Slot) -> bool {
        self.feature == feature && self.slot == slot
    }

    pub fn held(&self) -> impl Iterator<Item = Pickable> + '_ {
        self.pending.iter().flatten().copied()
    }

    pub fn holds(&self, pickable: Pickable) -> bool {
        self.held().any(|held| held == pickable)
    }

    pub fn hold(&mut self, pickable: Pickable) {
        if let Some(slot) = self.pending.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(pickable);
        }
    }

    fn held_count(&self) -> usize {
        self.held().count()
    }
}

fn key() -> Id {
    Id::unique("reference-picking")
}

pub fn publish(ctx: &Context, picking: Option<Picking>) {
    ctx.data_mut(|data| match picking {
        Some(picking) => {
            data.insert_temp(key(), picking);
        }
        None => data.remove::<Picking>(key()),
    });
}

pub fn current(ctx: &Context) -> Option<Picking> {
    ctx.data(|data| data.get_temp::<Picking>(key()))
}

fn kind(model: &Model, feature: FeatureId) -> Option<&FeatureKind> {
    model
        .document()
        .feature(feature)
        .map(|feature| &feature.kind)
}

fn datum(model: &Model, feature: FeatureId) -> Option<&Datum> {
    match kind(model, feature) {
        Some(FeatureKind::Datum(datum)) => Some(datum),
        _ => None,
    }
}

pub fn prompt(model: &Model, picking: Picking) -> String {
    let circular = matches!(
        kind(model, picking.feature),
        Some(FeatureKind::Pattern(pattern)) if matches!(pattern.kind, PatternKind::Circular(_))
    );
    match picking.slot {
        Slot::RevolveAxis | Slot::DatumRotation | Slot::MoveAxis => {
            format!("Click {AXIS} to turn about")
        }
        Slot::ExtrudeTarget(_) => "Click a face or plane to extrude up to".to_owned(),
        Slot::HoleTarget => "Click a flat face or plane to drill up to".to_owned(),
        Slot::CoilAxis => format!(
            "Click {AXIS} or a sketch line to wind the coil around, or press Escape to leave it \
             where it is"
        ),
        Slot::HelixAxis => format!(
            "Click {AXIS} or a sketch line to wind the helix around, or press Escape to leave it \
             where it is"
        ),
        Slot::HolePlace => {
            "Click a flat face where the hole goes, or press Escape to leave it where it is"
                .to_owned()
        }
        Slot::HoleEdge(_) => {
            "Click a straight edge, a line of another sketch or an axis to measure the hole from"
                .to_owned()
        }
        Slot::HoleConcentric(_) => "Click a round edge, a circle or arc of another sketch, or a \
                                    point, to centre the hole on"
            .to_owned(),
        Slot::HoleAdd => {
            "Click the face where each further hole goes, or press Escape to stop adding them"
                .to_owned()
        }
        Slot::ShellOwnThickness => {
            "Click a face to give it its own thickness or the shell's again, or press Escape to \
             stop"
                .to_owned()
        }
        Slot::ExtrudeDirection => "Click a straight edge, an axis or a sketch line to extrude \
                                    along"
            .to_owned(),
        Slot::RevolveTarget => {
            "Click a flat face or plane through the axis to turn up to".to_owned()
        }
        Slot::StartPlane => {
            "Click a flat face or plane parallel to the sketch to start from".to_owned()
        }
        Slot::MirrorPlane => "Click a plane or flat face to mirror across".to_owned(),
        Slot::MateMoving(MatePart::Main) => {
            "Click the face, axis or point of the moving body to mate".to_owned()
        }
        Slot::MateMoving(MatePart::Axis) => {
            format!("Click {AXIS} of the moving body to line up")
        }
        Slot::MateTarget(MatePart::Main) => {
            "Click the face, plane, axis or point to mate onto".to_owned()
        }
        Slot::MateTarget(MatePart::Axis) => format!("Click {AXIS} to line the body up on"),
        Slot::SplitPlane => {
            "Click a plane, flat face, sketch curve or another body to split along".to_owned()
        }
        Slot::SplitDirection => {
            "Click a straight edge, an axis, a round face or a sketch line to carry the curves \
             along"
                .to_owned()
        }
        Slot::FrameOrigin => {
            "Click a corner, round edge, sphere or torus, sketch point or datum point for its \
             origin"
                .to_owned()
        }
        Slot::FrameAxis => format!("Click {AXIS} for its X axis"),
        Slot::ScaleCentre => {
            "Click a corner, round edge, sphere or torus, sketch point or datum point to scale \
             about"
                .to_owned()
        }
        Slot::ThreadFace => "Click the round face of a bore, shaft or boss to thread".to_owned(),
        Slot::SweepPath => {
            "Click the curves of a sketch, the edges of a body or a helix to follow, again to \
             leave one out; press Escape when the path is complete"
                .to_owned()
        }
        Slot::LoftProfiles => {
            "Click a region or a curve of each sketch, or a flat face of a body, to loft through, \
             in order, again to leave a sketch out; press Escape when every profile is chosen"
                .to_owned()
        }
        Slot::LoftEnd => {
            "Click a corner, round edge, sphere or torus, sketch point or datum point for the loft \
             to end at"
                .to_owned()
        }
        Slot::MeasuredItem(part) => part.prompt().to_owned(),
        Slot::FramePlane => "Click a plane or flat face for its XY plane".to_owned(),
        Slot::ImagePlace => "Click a plane or flat face where the picture goes".to_owned(),
        Slot::ImageCalibration => {
            image_tools::calibration_prompt(picking.held_count() > 0).to_owned()
        }
        Slot::PrimitivePlace => {
            let noun = kind(model, picking.feature)
                .and_then(FeatureKind::primitive)
                .map_or("shape", |primitive| primitive.shape.kind().noun());
            format!(
                "Click a plane or flat face where the {noun} goes, or press Escape to leave it \
                 where it is"
            )
        }
        Slot::PatternDirection if circular => format!("Click {AXIS} to turn about"),
        Slot::PatternDirection => format!("Click {AXIS} to repeat along"),
        Slot::PatternSecond => format!("Click {AXIS} to also repeat along"),
        Slot::PatternPath => match kind(model, picking.feature) {
            Some(FeatureKind::Pattern(pattern))
                if matches!(pattern.kind, PatternKind::Points(_)) =>
            {
                "Click a point of the sketch whose points the copies go to".to_owned()
            }
            _ => "Click a curve of the sketch for the copies to follow".to_owned(),
        },
        Slot::PatternBase => {
            "Click a corner, round edge, sphere or torus, sketch point or datum point to move the \
             copies from"
                .to_owned()
        }
        Slot::DatumBase if picking.held_count() > 0 => held_prompt(model, picking),
        Slot::DatumBase => match datum(model, picking.feature) {
            Some(Datum::Axis(_)) => format!(
                "Click {AXIS} to run along, two planes that cross, two points, or a plane and a \
                 point"
            ),
            Some(Datum::PlaneThrough(_)) => {
                "Click three points, two planes to lie midway between, an axis and a point, two \
                 lines in one plane, or a round or curved edge to stand square to"
                    .to_owned()
            }
            Some(Datum::Point(_) | Datum::PointBy(_)) => {
                "Click a corner, round edge, sphere or torus, sketch point or datum point to \
                 place it at, a straight or curved edge to measure along, or lines and planes \
                 that meet"
                    .to_owned()
            }
            Some(Datum::Plane(_) | Datum::Frame(_)) | None => {
                "Click a plane or flat face to start from".to_owned()
            }
        },
    }
}

fn held_prompt(model: &Model, picking: Picking) -> String {
    let index = model.document().feature_index(picking.feature).unwrap_or(0);
    let held_plane = picking
        .held()
        .any(|held| datum_tools::plane_reference(model, held, index).is_some());
    match datum(model, picking.feature) {
        Some(Datum::Axis(_)) if held_plane => {
            "Click a second plane or flat face that crosses the first, or a point to stand \
             square on it"
                .to_owned()
        }
        Some(Datum::Axis(_)) => {
            "Click a second point, or a plane or flat face to stand square on".to_owned()
        }
        _ => "Click the next point, plane or axis to finish choosing".to_owned(),
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ChangeError {
    #[error("{}", GONE)]
    Gone { feature: FeatureId },
    #[error("{name} no longer takes this reference")]
    NotTaken { name: String },
    #[error(transparent)]
    Coil(#[from] coil_tools::CoilError),
    #[error(transparent)]
    Datum(#[from] datum_panel::DatumError),
    #[error(transparent)]
    Helix(#[from] helix_tools::HelixError),
    #[error(transparent)]
    HolePanel(#[from] hole_panel::HolePanelError),
    #[error(transparent)]
    HolePlacement(#[from] hole_placement::HolePlacementError),
    #[error(transparent)]
    Hole(#[from] hole_tools::HoleToolError),
    #[error(transparent)]
    Image(#[from] image_tools::ImageError),
    #[error(transparent)]
    Loft(#[from] loft_tools::LoftError),
    #[error(transparent)]
    Mate(#[from] mate_tools::MateError),
    #[error(transparent)]
    Measurement(#[from] measurement_tools::MeasurementError),
    #[error(transparent)]
    Mirror(#[from] mirror_tools::MirrorToolError),
    #[error(transparent)]
    Move(#[from] move_tools::MoveError),
    #[error(transparent)]
    Pattern(#[from] pattern_tools::PatternToolError),
    #[error(transparent)]
    Pipe(#[from] pipe_tools::PipeError),
    #[error(transparent)]
    Primitive(#[from] primitive_tools::PrimitiveError),
    #[error(transparent)]
    Scale(#[from] scale_tools::ScaleError),
    #[error(transparent)]
    Shell(#[from] shell_tools::ShellError),
    #[error(transparent)]
    SketchPattern(#[from] sketch_pattern_tools::PatternSketchError),
    #[error(transparent)]
    Solid(#[from] solid_panel::SolidChangeError),
    #[error(transparent)]
    SplitFace(#[from] split_face_tools::SplitFaceError),
    #[error(transparent)]
    Split(#[from] split_tools::SplitError),
    #[error(transparent)]
    Sweep(#[from] sweep_tools::SweepError),
    #[error(transparent)]
    Thread(#[from] thread_tools::ThreadToolError),
}

pub fn change(
    model: &Model,
    feature: FeatureId,
    slot: Slot,
    selection: &Selection,
) -> Result<Transaction, ChangeError> {
    let owner = model
        .document()
        .feature(feature)
        .ok_or(ChangeError::Gone { feature })?;
    match (slot, &owner.kind) {
        (Slot::RevolveAxis, FeatureKind::Solid(SolidFeature::Revolve(revolve))) => {
            solid_panel::selected_axis_change(model, selection, feature, revolve)
                .map_err(ChangeError::from)
        }
        (Slot::ExtrudeTarget(side), FeatureKind::Solid(SolidFeature::Extrude(extrude))) => {
            solid_panel::target_change(model, selection, feature, extrude, side)
                .map_err(ChangeError::from)
        }
        (Slot::StartPlane, FeatureKind::Solid(solid)) => {
            solid_panel::start_change(model, selection, feature, solid).map_err(ChangeError::from)
        }
        (Slot::SweepPath, FeatureKind::Solid(SolidFeature::Sweep(sweep))) => {
            sweep_tools::path_change(model, selection, feature, sweep).map_err(ChangeError::from)
        }
        (Slot::SweepPath, FeatureKind::Pipe(pipe)) => {
            pipe_tools::path_change(model, selection, feature, pipe).map_err(ChangeError::from)
        }
        (Slot::LoftProfiles, FeatureKind::Loft(loft)) => {
            loft_tools::profile_change(model, selection, feature, loft).map_err(ChangeError::from)
        }
        (Slot::LoftEnd, FeatureKind::Loft(loft)) => {
            loft_tools::end_change(model, selection, feature, loft).map_err(ChangeError::from)
        }
        (Slot::ExtrudeDirection, FeatureKind::Solid(SolidFeature::Extrude(extrude))) => {
            solid_panel::direction_change(model, selection, feature, extrude)
                .map_err(ChangeError::from)
        }
        (Slot::RevolveTarget, FeatureKind::Solid(SolidFeature::Revolve(revolve))) => {
            solid_panel::revolve_target_change(model, selection, feature, revolve)
                .map_err(ChangeError::from)
        }
        (Slot::HoleTarget, FeatureKind::Hole(hole)) => {
            hole_panel::target_change(model, selection, feature, hole).map_err(ChangeError::from)
        }
        (Slot::HolePlace, FeatureKind::Hole(hole)) => {
            hole_tools::place_change(model, selection, feature, hole).map_err(ChangeError::from)
        }
        (Slot::HoleEdge(point), FeatureKind::Hole(hole)) => {
            hole_placement::edge_change(model, selection, feature, hole, point)
                .map_err(ChangeError::from)
        }
        (Slot::HoleConcentric(point), FeatureKind::Hole(hole)) => {
            hole_placement::concentric_change(model, selection, feature, hole, point)
                .map_err(ChangeError::from)
        }
        (Slot::HoleAdd, FeatureKind::Hole(hole)) => {
            hole_placement::add_change(model, selection, feature, hole).map_err(ChangeError::from)
        }
        (Slot::ShellOwnThickness, FeatureKind::Shell(_)) => {
            shell_tools::own_thickness_change(model, selection, feature).map_err(ChangeError::from)
        }
        (Slot::MirrorPlane, FeatureKind::Mirror(mirror)) => {
            mirror_tools::plane_change(model, selection, feature, mirror).map_err(ChangeError::from)
        }
        (Slot::SplitPlane, FeatureKind::Split(split)) => {
            split_tools::along_change(model, selection, feature, split).map_err(ChangeError::from)
        }
        (Slot::SplitPlane, FeatureKind::SplitFace(split)) => {
            split_face_tools::along_change(model, selection, feature, split)
                .map_err(ChangeError::from)
        }
        (Slot::SplitDirection, FeatureKind::SplitFace(split)) => {
            split_face_tools::direction_change(model, selection, feature, split)
                .map_err(ChangeError::from)
        }
        (Slot::PatternDirection, FeatureKind::Pattern(pattern)) => {
            pattern_tools::selected_change(model, selection, feature, pattern, Reference::First)
                .map_err(ChangeError::from)
        }
        (Slot::PatternSecond, FeatureKind::Pattern(pattern)) => {
            pattern_tools::selected_change(model, selection, feature, pattern, Reference::Second)
                .map_err(ChangeError::from)
        }
        (Slot::PatternPath, FeatureKind::Pattern(pattern)) => {
            sketch_pattern_tools::sketch_change(model, selection, feature, pattern)
                .map_err(ChangeError::from)
        }
        (Slot::PatternBase, FeatureKind::Pattern(pattern)) => {
            sketch_pattern_tools::base_change(model, selection, feature, pattern)
                .map_err(ChangeError::from)
        }
        (Slot::DatumBase, FeatureKind::Datum(datum)) => {
            datum_panel::base_change(model, selection, feature, datum).map_err(ChangeError::from)
        }
        (Slot::DatumRotation, FeatureKind::Datum(datum)) => {
            datum_panel::rotation_change(model, selection, feature, datum)
                .map_err(ChangeError::from)
        }
        (Slot::FrameOrigin, FeatureKind::Datum(datum)) => {
            datum_panel::frame_change(model, selection, feature, datum, FramePart::Origin)
                .map_err(ChangeError::from)
        }
        (Slot::FrameAxis, FeatureKind::Datum(datum)) => {
            datum_panel::frame_change(model, selection, feature, datum, FramePart::XAxis)
                .map_err(ChangeError::from)
        }
        (Slot::FramePlane, FeatureKind::Datum(datum)) => {
            datum_panel::frame_change(model, selection, feature, datum, FramePart::Plane)
                .map_err(ChangeError::from)
        }
        (Slot::MoveAxis, FeatureKind::Move(movement)) => {
            move_tools::axis_change(model, selection, feature, movement).map_err(ChangeError::from)
        }
        (Slot::CoilAxis, FeatureKind::Coil(coil)) => {
            coil_tools::axis_change(model, selection, feature, coil).map_err(ChangeError::from)
        }
        (Slot::HelixAxis, FeatureKind::Helix(helix)) => {
            helix_tools::axis_change(model, selection, feature, helix).map_err(ChangeError::from)
        }
        (Slot::PrimitivePlace, FeatureKind::Primitive(primitive)) => {
            primitive_tools::place_change(model, selection, feature, primitive)
                .map_err(ChangeError::from)
        }
        (Slot::ImagePlace, FeatureKind::ReferenceImage(image)) => {
            image_tools::place_change(model, selection, feature, image).map_err(ChangeError::from)
        }
        (Slot::MateMoving(part), FeatureKind::Mate(mate)) => {
            mate_tools::moving_change(model, selection, feature, mate, part)
                .map_err(ChangeError::from)
        }
        (Slot::MateTarget(part), FeatureKind::Mate(mate)) => {
            mate_tools::target_change(model, selection, feature, mate, part)
                .map_err(ChangeError::from)
        }
        (Slot::ScaleCentre, FeatureKind::Scale(scale)) => {
            scale_tools::centre_change(model, selection, feature, scale).map_err(ChangeError::from)
        }
        (Slot::ThreadFace, FeatureKind::Thread(thread)) => {
            thread_tools::face_change(model, selection, feature, thread).map_err(ChangeError::from)
        }
        (Slot::MeasuredItem(part), FeatureKind::Measurement(measurement)) => {
            measurement_tools::item_change(model, selection, feature, measurement, part)
                .map_err(ChangeError::from)
        }
        _ => Err(ChangeError::NotTaken {
            name: owner.name.clone(),
        }),
    }
}

fn may_hold(model: &Model, picking: Picking, pickable: Pickable) -> bool {
    if picking.slot != Slot::DatumBase || picking.held_count() >= MAX_HELD {
        return false;
    }
    let index = model.document().feature_index(picking.feature).unwrap_or(0);
    let plane = || datum_tools::plane_reference(model, pickable, index).is_some();
    let axis = || datum_tools::axis_reference(model, pickable, index).is_some();
    let point = || datum_tools::point_reference(model, pickable, index).is_some();
    match datum(model, picking.feature) {
        Some(Datum::Axis(_)) => picking.held_count() == 0 && (plane() || point()),
        Some(Datum::PlaneThrough(_)) => plane() || axis() || point(),
        Some(Datum::PointBy(_)) => plane() || axis() || point(),
        Some(Datum::Plane(_) | Datum::Point(_) | Datum::Frame(_)) | None => false,
    }
}

pub fn click(model: &Model, picking: Picking, pickable: Pickable) -> Vec<Action> {
    if picking.holds(pickable) {
        return vec![Action::Editing(EditingCommand::Pick(Picking::new(
            picking.feature,
            picking.slot,
        )))];
    }
    let mut selection = Selection::default();
    for held in picking.held() {
        selection.toggle(held);
    }
    selection.toggle(pickable);
    match change(model, picking.feature, picking.slot, &selection) {
        Ok(transaction) if picking.slot.keeps_choosing() => vec![Action::Apply(transaction)],
        Ok(transaction) => vec![
            Action::Apply(transaction),
            Action::Editing(EditingCommand::StopPicking),
        ],
        Err(_) if may_hold(model, picking, pickable) => {
            vec![Action::Editing(EditingCommand::HoldPicked(pickable))]
        }
        Err(reason) => vec![Action::Inform(Notice::warning(reason.to_string()))],
    }
}
