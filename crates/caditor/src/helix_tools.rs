use caditor_document::{
    AxisReference, CoilLength, CoilSizing, CoilTurning, DEFAULT_PITCH, DEFAULT_TURNS, Document,
    Edit, Evaluation, FeatureId, FeatureKind, FeatureResult, Helix, HelixResult, PointReference,
    Transaction,
};
use caditor_expression::Expression;
use caditor_geometry::Point3;

use crate::{
    coil_tools::{self, CoilStart, DEFAULT_AXIS, DEFAULT_DIAMETER},
    datum_tools,
    editing::{self, EditingCommand},
    feature_fields::ChoiceRefusal,
    field::{self, CheckError},
    model::{Action, Model},
    reference_picking::{Picking, Slot},
    selection::Selection,
    units::LengthUnit,
};

pub const TITLE: &str = "Helix";
const POINTS_PER_TURN: usize = 64;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum HelixError {
    #[error("The feature no longer exists")]
    Gone { feature: FeatureId },
    #[error("Select an axis, straight edge, round face or sketch line to wind around")]
    NoAxis,
    #[error("The helix already winds around that axis")]
    AlreadyAround,
    #[error("Select a corner, round edge, sketch point or datum point made before the helix")]
    NoStartPoint,
    #[error("The helix already starts at that point")]
    AlreadyStartsThere,
    #[error(transparent)]
    AxisSelection(datum_tools::OneChoiceError),
    #[error(transparent)]
    PointSelection(datum_tools::OneChoiceError),
    #[error(transparent)]
    Choice(#[from] ChoiceRefusal),
    #[error(transparent)]
    Check(#[from] CheckError),
}

pub fn create(
    document: &Document,
    start: &CoilStart,
    unit: LengthUnit,
) -> (Transaction, FeatureId) {
    let name = editing::next_feature_name(document, TITLE);
    let helix = Helix {
        length: CoilLength::TurnsAndPitch {
            turns: Expression::number(DEFAULT_TURNS),
            pitch: unit.default_length(DEFAULT_PITCH),
        },
        ..Helix::new(
            start.axis.clone().unwrap_or(DEFAULT_AXIS),
            unit.default_length(DEFAULT_DIAMETER),
        )
    };
    let mut transaction = document.transaction(format!("Create {name}"));
    let feature = transaction.add_feature(name, FeatureKind::Helix(helix));
    (transaction.finish(), feature)
}

pub fn create_actions(model: &Model, start: &CoilStart) -> Vec<Action> {
    let (transaction, feature) = create(model.document(), start, model.length_unit());
    let mut actions = vec![
        Action::Apply(transaction),
        Action::Editing(EditingCommand::OpenSolid(feature)),
    ];
    if start.axis.is_none() {
        actions.push(Action::Editing(EditingCommand::Pick(Picking::new(
            feature,
            Slot::HelixAxis,
        ))));
    }
    actions
}

pub fn edit(document: &Document, feature: FeatureId, helix: Helix) -> Option<Transaction> {
    let name = &document.feature(feature)?.name;
    let transaction = Transaction::single(
        format!("Edit {name}"),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Helix(helix),
        },
    );
    Some(document.carrying_owners(transaction, feature, []))
}

pub fn change(model: &Model, feature: FeatureId, helix: Helix) -> Result<Transaction, HelixError> {
    let document = model.document();
    let transaction = edit(document, feature, helix).ok_or(HelixError::Gone { feature })?;
    field::checked(document, transaction)
}

pub fn with_axis(helix: &Helix, axis: AxisReference) -> Helix {
    Helix {
        axis,
        ..helix.clone()
    }
}

pub fn axis_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    helix: &Helix,
) -> Result<Transaction, HelixError> {
    let index = model
        .document()
        .feature_index(feature)
        .ok_or(HelixError::Gone { feature })?;
    let axis = datum_tools::only_axis(model, selection, index)
        .map_err(HelixError::AxisSelection)?
        .ok_or(HelixError::NoAxis)?;
    if axis == helix.axis {
        return Err(HelixError::AlreadyAround);
    }
    change(model, feature, with_axis(helix, axis))
}

pub fn with_start_point(helix: &Helix, start_point: Option<PointReference>) -> Helix {
    Helix {
        start_point,
        ..helix.clone()
    }
}

pub fn start_point_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    helix: &Helix,
) -> Result<Transaction, HelixError> {
    let index = model
        .document()
        .feature_index(feature)
        .ok_or(HelixError::Gone { feature })?;
    let point = datum_tools::only_point(model, selection, index)
        .map_err(HelixError::PointSelection)?
        .ok_or(HelixError::NoStartPoint)?;
    if helix.start_point.as_ref() == Some(&point) {
        return Err(HelixError::AlreadyStartsThere);
    }
    change(model, feature, with_start_point(helix, Some(point)))
}

pub fn reversed(helix: &Helix) -> Helix {
    Helix {
        reversed: !helix.reversed,
        ..helix.clone()
    }
}

pub fn with_turning(helix: &Helix, turning: CoilTurning) -> Helix {
    Helix {
        turning,
        ..helix.clone()
    }
}

pub fn sized_by(model: &Model, helix: &Helix, sizing: CoilSizing) -> Helix {
    Helix {
        length: coil_tools::sized_length(model, &helix.length, sizing),
        ..helix.clone()
    }
}

fn result(evaluation: &Evaluation, feature: FeatureId) -> Option<&HelixResult> {
    evaluation
        .feature(feature)
        .and_then(|status| status.result.as_deref())
        .and_then(FeatureResult::helix)
}

pub fn is_built(evaluation: &Evaluation, feature: FeatureId) -> bool {
    result(evaluation, feature).is_some()
}

pub fn points(evaluation: &Evaluation, feature: FeatureId) -> Vec<Point3> {
    let Some(result) = result(evaluation, feature) else {
        return Vec::new();
    };
    let mut points = Vec::with_capacity(result.turns.len() * POINTS_PER_TURN + 1);
    for (index, turn) in result.turns.iter().enumerate() {
        let range = turn.range();
        let first = usize::from(index > 0);
        points.extend((first..=POINTS_PER_TURN).map(|step| {
            turn.curve()
                .point(range.at(step as f64 / POINTS_PER_TURN as f64))
        }));
    }
    points
}

pub fn is_helix(document: &Document, feature: FeatureId) -> bool {
    document
        .feature(feature)
        .is_some_and(|feature| feature.kind.helix().is_some())
}
