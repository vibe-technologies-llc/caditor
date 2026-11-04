use caditor_document::{
    AxisReference, CoilLength, CoilSizing, CoilSpan, CoilTurning, DEFAULT_PITCH, DEFAULT_TURNS,
    Document, Edit, Evaluation, FeatureId, FeatureKind, FeatureResult, Helix, HelixKind,
    HelixResult, MAX_PITCH_ROWS, PitchRow, PointReference, Transaction, span_with_pitch,
};
use caditor_expression::{Dimension, Expression, Unit};
use caditor_geometry::Point3;
use caditor_kernel::{HelixEnds, HelixPitch, PitchStop};

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
pub const SPIRAL_TITLE: &str = "Spiral";
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

pub fn title(kind: HelixKind) -> &'static str {
    match kind {
        HelixKind::Helix => TITLE,
        HelixKind::FlatSpiral => SPIRAL_TITLE,
    }
}

pub fn create(
    document: &Document,
    start: &CoilStart,
    unit: LengthUnit,
    kind: HelixKind,
) -> (Transaction, FeatureId) {
    let name = editing::next_feature_name(document, title(kind));
    let helix = Helix {
        kind,
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

pub fn create_actions(model: &Model, start: &CoilStart, kind: HelixKind) -> Vec<Action> {
    let (transaction, feature) = create(model.document(), start, model.length_unit(), kind);
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

pub fn with_closed_ends(helix: &Helix, closed: bool) -> Helix {
    Helix {
        ends: closed.then(Box::default),
        pitch_table: if closed {
            Vec::new()
        } else {
            helix.pitch_table.clone()
        },
        ..helix.clone()
    }
}

pub fn with_progression(model: &Model, helix: &Helix, progressive: bool) -> Helix {
    let final_pitch = progressive.then(|| {
        let start = coil_tools::evaluated_span(model, &helix.length)
            .map_or(DEFAULT_PITCH, |span| span.pitch);
        Expression::measure(2.0 * start, Unit::Millimetre)
    });
    Helix {
        final_pitch,
        pitch_table: Vec::new(),
        ..helix.clone()
    }
}

pub fn with_pitch_table(model: &Model, helix: &Helix, tabled: bool) -> Helix {
    let pitch_table = if tabled {
        let (start, turns) = coil_tools::evaluated_span(model, &helix.length)
            .map_or((DEFAULT_PITCH, DEFAULT_TURNS), |span| {
                (span.pitch, span.turns)
            });
        vec![
            PitchRow::new(
                Expression::number((0.5 * turns * 100.0).round() / 100.0),
                Expression::measure(start, Unit::Millimetre),
            ),
            PitchRow::new(
                Expression::number((turns * 100.0).round() / 100.0),
                Expression::measure(2.0 * start, Unit::Millimetre),
            ),
        ]
    } else {
        Vec::new()
    };
    Helix {
        ends: if tabled { None } else { helix.ends.clone() },
        final_pitch: None,
        pitch_table,
        ..helix.clone()
    }
}

pub fn can_add_pitch_row(helix: &Helix) -> bool {
    helix.pitch_table.len() < MAX_PITCH_ROWS
}

pub fn with_pitch_row_added(model: &Model, helix: &Helix) -> Helix {
    let turns =
        coil_tools::evaluated_span(model, &helix.length).map_or(DEFAULT_TURNS, |span| span.turns);
    let last = helix.pitch_table.last();
    let after = last
        .and_then(|row| number(model, &row.turns))
        .unwrap_or(0.0);
    let row = PitchRow::new(
        Expression::number((0.5 * (after + turns.max(after)) * 100.0).round() / 100.0),
        last.map_or_else(
            || Expression::measure(DEFAULT_PITCH, Unit::Millimetre),
            |row| row.pitch.clone(),
        ),
    );
    let mut pitch_table = helix.pitch_table.clone();
    pitch_table.push(row);
    Helix {
        pitch_table,
        ..helix.clone()
    }
}

pub fn with_last_pitch_row_removed(helix: &Helix) -> Helix {
    let mut pitch_table = helix.pitch_table.clone();
    pitch_table.pop();
    Helix {
        pitch_table,
        ..helix.clone()
    }
}

pub fn with_pitch_row(helix: &Helix, index: usize, turns: bool, value: Expression) -> Helix {
    let mut pitch_table = helix.pitch_table.clone();
    if let Some(row) = pitch_table.get_mut(index) {
        if turns {
            row.turns = value;
        } else {
            row.pitch = value;
        }
    }
    Helix {
        pitch_table,
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
    let length = if helix.is_flat() {
        coil_tools::resized_length(&helix.length, sizing, spiral_span(model, helix))
    } else if let (Some(span), Some(plan)) = (
        coil_tools::evaluated_span(model, &helix.length),
        evaluated_plan(model, helix).filter(|plan| *plan != HelixPitch::default()),
    ) {
        let shaped = span_with_pitch(helix.length.sizing(), span, plan);
        coil_tools::resized_length(&helix.length, sizing, shaped)
    } else {
        coil_tools::sized_length(model, &helix.length, sizing)
    };
    Helix {
        length,
        ..helix.clone()
    }
}

fn value(model: &Model, expression: Option<&Expression>, dimension: Dimension) -> Option<f64> {
    let parameters = &model.evaluation().parameters;
    expression?
        .evaluate_as(dimension, &|id| parameters.value(id))
        .ok()
        .filter(|value| value.is_finite() && *value > 0.0)
}

fn number(model: &Model, expression: &Expression) -> Option<f64> {
    let parameters = &model.evaluation().parameters;
    expression
        .evaluate_as(Dimension::NONE, &|id| parameters.value(id))
        .ok()
        .filter(|value| value.is_finite() && *value >= 0.0)
}

pub fn evaluated_plan(model: &Model, helix: &Helix) -> Option<HelixPitch> {
    if helix.is_flat() {
        return Some(HelixPitch::default());
    }
    let number = |expression: &Expression| number(model, expression);
    let ends = match &helix.ends {
        None => None,
        Some(ends) => Some(HelixEnds {
            turns: number(&ends.turns)?,
            pitch: value(model, Some(&ends.pitch), Dimension::LENGTH)?,
            change: number(&ends.change)?,
        }),
    };
    let final_pitch = match &helix.final_pitch {
        None => None,
        Some(pitch) => Some(value(model, Some(pitch), Dimension::LENGTH)?),
    };
    let table = helix
        .pitch_table
        .iter()
        .map(|row| {
            Some(PitchStop {
                turns: number(&row.turns)?,
                pitch: value(model, Some(&row.pitch), Dimension::LENGTH)?,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(HelixPitch {
        ends,
        final_pitch,
        table,
    })
}

fn spiral_span(model: &Model, helix: &Helix) -> CoilSpan {
    let length = &helix.length;
    let inner = value(model, Some(&helix.diameter), Dimension::LENGTH).unwrap_or(DEFAULT_DIAMETER);
    let width = value(model, length.height(), Dimension::LENGTH)
        .map(|outer| 0.5 * (outer - inner))
        .filter(|width| *width > 0.0);
    let pitch = value(model, length.pitch(), Dimension::LENGTH);
    let turns = value(model, length.turns(), Dimension::NONE);
    let (pitch, turns) = match (width, pitch, turns) {
        (Some(width), Some(pitch), _) => (pitch, width / pitch),
        (Some(width), None, Some(turns)) => (width / turns, turns),
        (_, Some(pitch), Some(turns)) => (pitch, turns),
        _ => (DEFAULT_PITCH, DEFAULT_TURNS),
    };
    CoilSpan {
        height: inner + 2.0 * pitch * turns,
        pitch,
        turns,
    }
}

pub fn with_kind(model: &Model, helix: &Helix, kind: HelixKind) -> Helix {
    let turned = sized_by(model, helix, CoilSizing::TurnsAndPitch);
    let ends = match kind {
        HelixKind::Helix => turned.ends.clone(),
        HelixKind::FlatSpiral => None,
    };
    Helix {
        kind,
        ends,
        ..turned
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
