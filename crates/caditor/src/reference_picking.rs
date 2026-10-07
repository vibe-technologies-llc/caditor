use caditor_document::{Datum, FeatureId, FeatureKind, PatternKind, SolidFeature, Transaction};
use egui::{Context, Id};

use crate::{
    datum_panel, datum_tools,
    editing::EditingCommand,
    mirror_tools,
    model::{Action, Model, Notice},
    pattern_tools::{self, Reference},
    selection::{Pickable, Selection},
    solid_panel,
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
    MirrorPlane,
    PatternDirection,
    PatternSecond,
    DatumBase,
    DatumRotation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Picking {
    pub feature: FeatureId,
    pub slot: Slot,
    pub pending: Option<Pickable>,
}

impl Picking {
    pub fn new(feature: FeatureId, slot: Slot) -> Self {
        Self {
            feature,
            slot,
            pending: None,
        }
    }

    pub fn is_for(&self, feature: FeatureId, slot: Slot) -> bool {
        self.feature == feature && self.slot == slot
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

fn axis_datum(model: &Model, feature: FeatureId) -> bool {
    matches!(
        kind(model, feature),
        Some(FeatureKind::Datum(Datum::Axis(_)))
    )
}

pub fn prompt(model: &Model, picking: Picking) -> String {
    let circular = matches!(
        kind(model, picking.feature),
        Some(FeatureKind::Pattern(pattern)) if matches!(pattern.kind, PatternKind::Circular(_))
    );
    match picking.slot {
        Slot::RevolveAxis | Slot::DatumRotation => format!("Click {AXIS} to turn about"),
        Slot::ExtrudeTarget(_) => "Click a flat face or plane to extrude up to".to_owned(),
        Slot::StartPlane => {
            "Click a flat face or plane parallel to the sketch to start from".to_owned()
        }
        Slot::MirrorPlane => "Click a plane or flat face to mirror across".to_owned(),
        Slot::PatternDirection if circular => format!("Click {AXIS} to turn about"),
        Slot::PatternDirection => format!("Click {AXIS} to repeat along"),
        Slot::PatternSecond => format!("Click {AXIS} to also repeat along"),
        Slot::DatumBase if picking.pending.is_some() => {
            "Click a second plane or flat face that crosses the first".to_owned()
        }
        Slot::DatumBase if axis_datum(model, picking.feature) => {
            format!("Click {AXIS} to run along, or two planes or flat faces that cross")
        }
        Slot::DatumBase => "Click a plane or flat face to start from".to_owned(),
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
        (Slot::MirrorPlane, FeatureKind::Mirror(mirror)) => {
            mirror_tools::plane_change(model, selection, feature, mirror)
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
        _ => Err(format!("{} no longer takes this reference", owner.name)),
    }
}

fn holds(model: &Model, picking: Picking, pickable: Pickable) -> bool {
    let index = model.document().feature_index(picking.feature).unwrap_or(0);
    picking.slot == Slot::DatumBase
        && picking.pending.is_none()
        && axis_datum(model, picking.feature)
        && datum_tools::plane_reference(model, pickable, index).is_some()
}

pub fn click(model: &Model, picking: Picking, pickable: Pickable) -> Vec<Action> {
    if picking.pending == Some(pickable) {
        return vec![Action::Editing(EditingCommand::Pick(Picking::new(
            picking.feature,
            picking.slot,
        )))];
    }
    let mut selection = Selection::default();
    selection.replace_with(pickable);
    if let Some(pending) = picking.pending {
        selection.toggle(pending);
    }
    match change(model, picking.feature, picking.slot, &selection) {
        Ok(transaction) => vec![
            Action::Apply(transaction),
            Action::Editing(EditingCommand::StopPicking),
        ],
        Err(_) if holds(model, picking, pickable) => {
            vec![Action::Editing(EditingCommand::HoldPicked(pickable))]
        }
        Err(reason) => vec![Action::Inform(Notice::info(reason))],
    }
}
