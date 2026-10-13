use caditor_document::{
    AxisReference, BodyOperation, Coil, CoilLength, CoilSizing, CoilSpan, CoilTurning,
    DEFAULT_PITCH, DEFAULT_TURNS, Document, Edit, FeatureId, FeatureKind, PrincipalAxis,
    Transaction,
};
use caditor_expression::{Dimension, Expression};
use caditor_kernel::{CoilPlacement, CoilShape};

use crate::{
    datum_tools,
    editing::{self, EditingCommand},
    field,
    model::{Action, Model},
    reference_picking::{Picking, Slot},
    selection::Selection,
    units::LengthUnit,
};

pub const TITLE: &str = "Coil";
pub const DEFAULT_DIAMETER: f64 = 20.0;
pub const DEFAULT_SECTION: f64 = 2.0;
pub const DEFAULT_AXIS: AxisReference = AxisReference::Principal(PrincipalAxis::Z);
const SHOWN_PLACES: f64 = 1e6;
const GONE: &str = "The feature no longer exists";
const NO_AXIS: &str = "Select an axis, straight edge, round face or sketch line to wind around";
const ALREADY_AROUND: &str = "The coil already winds around that axis";

#[derive(Debug, Clone, PartialEq)]
pub struct CoilStart {
    pub axis: Option<AxisReference>,
}

pub fn source(model: &Model, selection: &Selection) -> Result<CoilStart, &'static str> {
    let axis = datum_tools::only_axis(model, selection, model.document().bar_index())?;
    Ok(CoilStart { axis })
}

pub fn create(
    document: &Document,
    start: &CoilStart,
    unit: LengthUnit,
) -> (Transaction, FeatureId) {
    let name = editing::next_feature_name(document, TITLE);
    let coil = Coil {
        length: CoilLength::TurnsAndPitch {
            turns: Expression::number(DEFAULT_TURNS),
            pitch: unit.default_length(DEFAULT_PITCH),
        },
        ..Coil::new(
            start.axis.clone().unwrap_or(DEFAULT_AXIS),
            unit.default_length(DEFAULT_DIAMETER),
            unit.default_length(DEFAULT_SECTION),
        )
    };
    let mut transaction = document.transaction(format!("Create {name}"));
    let feature = transaction.add_feature(name, FeatureKind::Coil(coil));
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
            Slot::CoilAxis,
        ))));
    }
    actions
}

pub fn edit(document: &Document, feature: FeatureId, coil: Coil) -> Option<Transaction> {
    let name = &document.feature(feature)?.name;
    let transaction = Transaction::single(
        format!("Edit {name}"),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Coil(coil),
        },
    );
    Some(document.carrying_owners(transaction, feature, []))
}

pub fn change(model: &Model, feature: FeatureId, coil: Coil) -> Result<Transaction, String> {
    let document = model.document();
    let transaction = edit(document, feature, coil).ok_or_else(|| GONE.to_owned())?;
    field::checked(document, transaction)
}

pub fn with_axis(coil: &Coil, axis: AxisReference) -> Coil {
    Coil {
        axis,
        ..coil.clone()
    }
}

pub fn axis_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    coil: &Coil,
) -> Result<Transaction, String> {
    let index = model
        .document()
        .feature_index(feature)
        .ok_or_else(|| GONE.to_owned())?;
    let axis = datum_tools::only_axis(model, selection, index)?.ok_or(NO_AXIS)?;
    if axis == coil.axis {
        return Err(ALREADY_AROUND.to_owned());
    }
    change(model, feature, with_axis(coil, axis))
}

pub fn reversed(coil: &Coil) -> Coil {
    Coil {
        reversed: !coil.reversed,
        ..coil.clone()
    }
}

pub fn with_operation(coil: &Coil, operation: BodyOperation) -> Coil {
    Coil {
        operation,
        ..coil.clone()
    }
}

pub fn with_shape(coil: &Coil, shape: CoilShape) -> Coil {
    Coil {
        shape,
        ..coil.clone()
    }
}

pub fn with_placement(coil: &Coil, placement: CoilPlacement) -> Coil {
    Coil {
        placement,
        ..coil.clone()
    }
}

pub fn with_turning(coil: &Coil, turning: CoilTurning) -> Coil {
    Coil {
        turning,
        ..coil.clone()
    }
}

pub fn evaluated_span(model: &Model, coil: &Coil) -> Option<CoilSpan> {
    let parameters = &model.evaluation().parameters;
    let value = |expression: Option<&Expression>, dimension: Dimension| {
        expression?
            .evaluate_as(dimension, &|id| parameters.value(id))
            .ok()
            .filter(|value| value.is_finite() && *value > 0.0)
    };
    let length = Dimension::LENGTH;
    let number = Dimension::NONE;
    let height = value(coil.length.height(), length);
    let pitch = value(coil.length.pitch(), length);
    let turns = value(coil.length.turns(), number);
    match (height, pitch, turns) {
        (Some(height), Some(pitch), _) => Some(CoilSpan {
            height,
            pitch,
            turns: height / pitch,
        }),
        (Some(height), None, Some(turns)) => Some(CoilSpan {
            height,
            pitch: height / turns,
            turns,
        }),
        (None, Some(pitch), Some(turns)) => Some(CoilSpan {
            height: pitch * turns,
            pitch,
            turns,
        }),
        _ => None,
    }
}

fn rounded(span: CoilSpan) -> CoilSpan {
    let round = |value: f64| (value * SHOWN_PLACES).round() / SHOWN_PLACES;
    CoilSpan {
        height: round(span.height),
        pitch: round(span.pitch),
        turns: round(span.turns),
    }
}

pub fn sized_by(model: &Model, coil: &Coil, sizing: CoilSizing) -> Coil {
    let span = evaluated_span(model, coil).unwrap_or(CoilSpan {
        height: DEFAULT_TURNS * DEFAULT_PITCH,
        pitch: DEFAULT_PITCH,
        turns: DEFAULT_TURNS,
    });
    let kept = |expression: Option<&Expression>| expression.cloned();
    let fresh = CoilLength::of(sizing, rounded(span));
    let length = match (sizing, &fresh) {
        (CoilSizing::TurnsAndPitch, CoilLength::TurnsAndPitch { turns, pitch }) => {
            CoilLength::TurnsAndPitch {
                turns: kept(coil.length.turns()).unwrap_or_else(|| turns.clone()),
                pitch: kept(coil.length.pitch()).unwrap_or_else(|| pitch.clone()),
            }
        }
        (CoilSizing::HeightAndTurns, CoilLength::HeightAndTurns { height, turns }) => {
            CoilLength::HeightAndTurns {
                height: kept(coil.length.height()).unwrap_or_else(|| height.clone()),
                turns: kept(coil.length.turns()).unwrap_or_else(|| turns.clone()),
            }
        }
        (CoilSizing::HeightAndPitch, CoilLength::HeightAndPitch { height, pitch }) => {
            CoilLength::HeightAndPitch {
                height: kept(coil.length.height()).unwrap_or_else(|| height.clone()),
                pitch: kept(coil.length.pitch()).unwrap_or_else(|| pitch.clone()),
            }
        }
        _ => fresh,
    };
    Coil {
        length,
        ..coil.clone()
    }
}
