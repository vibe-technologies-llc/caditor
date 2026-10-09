use caditor_document::{Datum, FeatureId, FeatureKind, PatternKind, SolidFeature, Transaction};
use egui::{Context, Id};

use crate::{
    datum_panel::{self, FramePart},
    datum_tools,
    editing::EditingCommand,
    hole_panel, mate_tools, mirror_tools,
    model::{Action, Model, Notice},
    move_tools,
    pattern_tools::{self, Reference},
    primitive_tools, scale_tools,
    selection::{Pickable, Selection},
    solid_panel, split_tools, thread_tools,
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
    RevolveTarget,
    ExtrudeDirection,
    MirrorPlane,
    SplitPlane,
    PatternDirection,
    PatternSecond,
    DatumBase,
    DatumRotation,
    MoveAxis,
    PrimitivePlace,
    MateMoving,
    MateTarget,
    FrameOrigin,
    FrameAxis,
    FramePlane,
    ScaleCentre,
    ThreadFace,
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
    Id::new("reference-picking")
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
        Slot::MateMoving => "Click the face or axis of the moving body to mate".to_owned(),
        Slot::MateTarget => "Click the face, plane or axis to mate onto".to_owned(),
        Slot::SplitPlane => {
            "Click a plane, flat face, sketch curve or another body to split along".to_owned()
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
        Slot::FramePlane => "Click a plane or flat face for its XY plane".to_owned(),
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

pub fn change(
    model: &Model,
    feature: FeatureId,
    slot: Slot,
    selection: &Selection,
) -> Result<Transaction, String> {
    let owner = model
        .document()
        .feature(feature)
        .ok_or_else(|| GONE.to_owned())?;
    match (slot, &owner.kind) {
        (Slot::RevolveAxis, FeatureKind::Solid(SolidFeature::Revolve(revolve))) => {
            solid_panel::selected_axis_change(model, selection, feature, revolve)
        }
        (Slot::ExtrudeTarget(side), FeatureKind::Solid(SolidFeature::Extrude(extrude))) => {
            solid_panel::target_change(model, selection, feature, extrude, side)
        }
        (Slot::StartPlane, FeatureKind::Solid(solid)) => {
            solid_panel::start_change(model, selection, feature, solid)
        }
        (Slot::ExtrudeDirection, FeatureKind::Solid(SolidFeature::Extrude(extrude))) => {
            solid_panel::direction_change(model, selection, feature, extrude)
        }
        (Slot::RevolveTarget, FeatureKind::Solid(SolidFeature::Revolve(revolve))) => {
            solid_panel::revolve_target_change(model, selection, feature, revolve)
        }
        (Slot::HoleTarget, FeatureKind::Hole(hole)) => {
            hole_panel::target_change(model, selection, feature, hole)
        }
        (Slot::MirrorPlane, FeatureKind::Mirror(mirror)) => {
            mirror_tools::plane_change(model, selection, feature, mirror)
        }
        (Slot::SplitPlane, FeatureKind::Split(split)) => {
            split_tools::along_change(model, selection, feature, split)
        }
        (Slot::PatternDirection, FeatureKind::Pattern(pattern)) => {
            pattern_tools::selected_change(model, selection, feature, pattern, Reference::First)
        }
        (Slot::PatternSecond, FeatureKind::Pattern(pattern)) => {
            pattern_tools::selected_change(model, selection, feature, pattern, Reference::Second)
        }
        (Slot::DatumBase, FeatureKind::Datum(datum)) => {
            datum_panel::base_change(model, selection, feature, datum)
        }
        (Slot::DatumRotation, FeatureKind::Datum(datum)) => {
            datum_panel::rotation_change(model, selection, feature, datum)
        }
        (Slot::FrameOrigin, FeatureKind::Datum(datum)) => {
            datum_panel::frame_change(model, selection, feature, datum, FramePart::Origin)
        }
        (Slot::FrameAxis, FeatureKind::Datum(datum)) => {
            datum_panel::frame_change(model, selection, feature, datum, FramePart::XAxis)
        }
        (Slot::FramePlane, FeatureKind::Datum(datum)) => {
            datum_panel::frame_change(model, selection, feature, datum, FramePart::Plane)
        }
        (Slot::MoveAxis, FeatureKind::Move(movement)) => {
            move_tools::axis_change(model, selection, feature, movement)
        }
        (Slot::PrimitivePlace, FeatureKind::Primitive(primitive)) => {
            primitive_tools::place_change(model, selection, feature, primitive)
        }
        (Slot::MateMoving, FeatureKind::Mate(mate)) => {
            mate_tools::moving_change(model, selection, feature, mate)
        }
        (Slot::MateTarget, FeatureKind::Mate(mate)) => {
            mate_tools::target_change(model, selection, feature, mate)
        }
        (Slot::ScaleCentre, FeatureKind::Scale(scale)) => {
            scale_tools::centre_change(model, selection, feature, scale)
        }
        (Slot::ThreadFace, FeatureKind::Thread(thread)) => {
            thread_tools::face_change(model, selection, feature, thread)
        }
        _ => Err(format!("{} no longer takes this reference", owner.name)),
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
        Ok(transaction) => vec![
            Action::Apply(transaction),
            Action::Editing(EditingCommand::StopPicking),
        ],
        Err(_) if may_hold(model, picking, pickable) => {
            vec![Action::Editing(EditingCommand::HoldPicked(pickable))]
        }
        Err(reason) => vec![Action::Inform(Notice::warning(reason))],
    }
}
