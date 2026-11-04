use std::collections::BTreeSet;

use caditor_expression::{Dimension, Expression, ParameterId, Unit};
use caditor_geometry::{Plane, Ray};
use caditor_kernel::{
    CoilError, CoilFace, CoilPlacement, CoilShape, CoilSpec, Handedness, LEAST_COIL_TURNS,
    MAX_COIL_TAPER_DEGREES, MAX_COIL_TURNS, coil,
};

use crate::{
    datum::AxisReference,
    document::{Feature, FeatureId},
    failure::Cause,
    helix::MAX_PITCH_ROWS,
    primitive::{Context, combined},
    recompute::{CancelToken, Failure, FeatureResult, Inputs},
    solid::BodyOperation,
};

pub const COIL_SHAPE: &str = "coil";
pub const DEFAULT_TURNS: f64 = 5.0;
pub const DEFAULT_PITCH: f64 = 5.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CoilTurning {
    CounterClockwise,
    Clockwise,
}

impl CoilTurning {
    pub const ALL: [Self; 2] = [Self::CounterClockwise, Self::Clockwise];

    pub fn name(self) -> &'static str {
        match self {
            Self::CounterClockwise => "Counter-clockwise",
            Self::Clockwise => "Clockwise",
        }
    }

    pub fn sign(self) -> f64 {
        match self {
            Self::CounterClockwise => 1.0,
            Self::Clockwise => -1.0,
        }
    }

    pub fn hand(self) -> Handedness {
        match self {
            Self::CounterClockwise => Handedness::Right,
            Self::Clockwise => Handedness::Left,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CoilSizing {
    TurnsAndPitch,
    HeightAndTurns,
    HeightAndPitch,
}

impl CoilSizing {
    pub const ALL: [Self; 3] = [
        Self::TurnsAndPitch,
        Self::HeightAndTurns,
        Self::HeightAndPitch,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::TurnsAndPitch => "Turns and pitch",
            Self::HeightAndTurns => "Height and turns",
            Self::HeightAndPitch => "Height and pitch",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CoilLength {
    TurnsAndPitch {
        turns: Expression,
        pitch: Expression,
    },
    HeightAndTurns {
        height: Expression,
        turns: Expression,
    },
    HeightAndPitch {
        height: Expression,
        pitch: Expression,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CoilSpan {
    pub height: f64,
    pub pitch: f64,
    pub turns: f64,
}

impl CoilLength {
    pub fn sizing(&self) -> CoilSizing {
        match self {
            Self::TurnsAndPitch { .. } => CoilSizing::TurnsAndPitch,
            Self::HeightAndTurns { .. } => CoilSizing::HeightAndTurns,
            Self::HeightAndPitch { .. } => CoilSizing::HeightAndPitch,
        }
    }

    pub fn of(sizing: CoilSizing, span: CoilSpan) -> Self {
        let length = |value: f64| Expression::measure(value, Unit::Millimetre);
        let turns = Expression::number(span.turns);
        match sizing {
            CoilSizing::TurnsAndPitch => Self::TurnsAndPitch {
                turns,
                pitch: length(span.pitch),
            },
            CoilSizing::HeightAndTurns => Self::HeightAndTurns {
                height: length(span.height),
                turns,
            },
            CoilSizing::HeightAndPitch => Self::HeightAndPitch {
                height: length(span.height),
                pitch: length(span.pitch),
            },
        }
    }

    pub fn height(&self) -> Option<&Expression> {
        match self {
            Self::HeightAndTurns { height, .. } | Self::HeightAndPitch { height, .. } => {
                Some(height)
            }
            Self::TurnsAndPitch { .. } => None,
        }
    }

    pub fn pitch(&self) -> Option<&Expression> {
        match self {
            Self::TurnsAndPitch { pitch, .. } | Self::HeightAndPitch { pitch, .. } => Some(pitch),
            Self::HeightAndTurns { .. } => None,
        }
    }

    pub fn turns(&self) -> Option<&Expression> {
        match self {
            Self::TurnsAndPitch { turns, .. } | Self::HeightAndTurns { turns, .. } => Some(turns),
            Self::HeightAndPitch { .. } => None,
        }
    }

    pub fn expressions(&self) -> [&Expression; 2] {
        match self {
            Self::TurnsAndPitch { turns, pitch } => [turns, pitch],
            Self::HeightAndTurns { height, turns } => [height, turns],
            Self::HeightAndPitch { height, pitch } => [height, pitch],
        }
    }

    pub fn expressions_mut(&mut self) -> [&mut Expression; 2] {
        match self {
            Self::TurnsAndPitch { turns, pitch } => [turns, pitch],
            Self::HeightAndTurns { height, turns } => [height, turns],
            Self::HeightAndPitch { height, pitch } => [height, pitch],
        }
    }

    pub fn lengths_mut(&mut self) -> Vec<&mut Expression> {
        match self {
            Self::TurnsAndPitch { pitch, .. } => vec![pitch],
            Self::HeightAndTurns { height, .. } => vec![height],
            Self::HeightAndPitch { height, pitch } => vec![height, pitch],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Coil {
    pub axis: AxisReference,
    pub reversed: bool,
    pub diameter: Expression,
    pub length: CoilLength,
    pub shape: CoilShape,
    pub size: Expression,
    pub placement: CoilPlacement,
    pub taper: Expression,
    pub turning: CoilTurning,
    pub operation: BodyOperation,
}

pub fn shape_name(shape: CoilShape) -> &'static str {
    match shape {
        CoilShape::Round => "Round",
        CoilShape::Square => "Square",
    }
}

pub fn size_name(shape: CoilShape) -> &'static str {
    match shape {
        CoilShape::Round => "section diameter",
        CoilShape::Square => "section width",
    }
}

pub fn placement_name(placement: CoilPlacement) -> &'static str {
    match placement {
        CoilPlacement::Inside => "Inside",
        CoilPlacement::Centred => "Centred",
        CoilPlacement::Outside => "Outside",
    }
}

pub fn part_name(shape: CoilShape, entity: u64) -> &'static str {
    match (CoilFace::of_entity(entity), shape) {
        (Some(CoilFace::Start), _) => "start",
        (Some(CoilFace::End), _) => "end",
        (Some(_), CoilShape::Round) | (None, _) => "surface",
        (Some(CoilFace::Bottom), CoilShape::Square) => "underside",
        (Some(CoilFace::Outside), CoilShape::Square) => "outside",
        (Some(CoilFace::Top), CoilShape::Square) => "top",
        (Some(CoilFace::Inside), CoilShape::Square) => "inside",
    }
}

impl Coil {
    pub fn new(axis: AxisReference, diameter: Expression, size: Expression) -> Self {
        Self {
            axis,
            reversed: false,
            diameter,
            length: CoilLength::TurnsAndPitch {
                turns: Expression::number(DEFAULT_TURNS),
                pitch: Expression::measure(DEFAULT_PITCH, Unit::Millimetre),
            },
            shape: CoilShape::Round,
            size,
            placement: CoilPlacement::Centred,
            taper: Expression::measure(0.0, Unit::Degree),
            turning: CoilTurning::CounterClockwise,
            operation: BodyOperation::NewBody,
        }
    }

    pub fn expressions(&self) -> Vec<&Expression> {
        let mut expressions = vec![&self.diameter, &self.size, &self.taper];
        expressions.extend(self.length.expressions());
        expressions
    }

    pub fn expressions_mut(&mut self) -> Vec<&mut Expression> {
        let mut expressions = vec![&mut self.diameter, &mut self.size, &mut self.taper];
        expressions.extend(self.length.expressions_mut());
        expressions
    }

    pub fn lengths_mut(&mut self) -> Vec<&mut Expression> {
        let mut lengths = vec![&mut self.diameter, &mut self.size];
        lengths.extend(self.length.lengths_mut());
        lengths
    }

    pub fn parameters(&self) -> BTreeSet<ParameterId> {
        self.expressions()
            .into_iter()
            .flat_map(Expression::parameters)
            .collect()
    }

    pub fn uses_parameter(&self, parameter: ParameterId) -> bool {
        self.expressions()
            .into_iter()
            .any(|expression| expression.uses(parameter))
    }

    pub fn features(&self) -> BTreeSet<FeatureId> {
        self.operation
            .target()
            .into_iter()
            .chain(self.axis.datum())
            .chain(self.axis.body())
            .chain(self.axis.sketch())
            .chain(self.axis.frame())
            .collect()
    }

    pub fn origin_features(&self) -> BTreeSet<FeatureId> {
        self.axis.origin_features()
    }

    pub fn heap_size(&self) -> usize {
        self.axis.heap_size()
            + self
                .expressions()
                .into_iter()
                .map(Expression::heap_size)
                .sum::<usize>()
    }

    pub fn frame(&self, axis: Ray) -> Option<Plane> {
        let direction = if self.reversed {
            -axis.direction()
        } else {
            axis.direction()
        };
        Plane::new(axis.origin(), direction)
    }
}

pub(crate) fn number(
    context: &Context<'_>,
    expression: &Expression,
    what: &str,
) -> Result<f64, Failure> {
    let value = context.value_as(expression, what, Dimension::NONE)?;
    if !(LEAST_COIL_TURNS..=MAX_COIL_TURNS).contains(&value) {
        let shape = context.shape();
        return Err(context.error(CoilCause::TurnsRange {
            shape: shape.to_owned(),
            what: what.to_owned(),
        }));
    }
    Ok(value)
}

pub(crate) fn span(context: &Context<'_>, length: &CoilLength) -> Result<CoilSpan, Failure> {
    match length {
        CoilLength::TurnsAndPitch { turns, pitch } => {
            let turns = number(context, turns, "turns")?;
            let pitch = context.size(pitch, "pitch")?;
            Ok(CoilSpan {
                height: pitch * turns,
                pitch,
                turns,
            })
        }
        CoilLength::HeightAndTurns { height, turns } => {
            let height = context.size(height, "height")?;
            let turns = number(context, turns, "turns")?;
            Ok(CoilSpan {
                height,
                pitch: height / turns,
                turns,
            })
        }
        CoilLength::HeightAndPitch { height, pitch } => {
            let height = context.size(height, "height")?;
            let pitch = context.size(pitch, "pitch")?;
            let turns = height / pitch;
            if !(LEAST_COIL_TURNS..=MAX_COIL_TURNS).contains(&turns) {
                let shape = context.shape();
                return Err(context.error(CoilCause::HeightPitchTurns {
                    shape: shape.to_owned(),
                    height,
                    pitch,
                    turns,
                }));
            }
            Ok(CoilSpan {
                height,
                pitch,
                turns,
            })
        }
    }
}

pub(crate) fn taper(context: &Context<'_>, expression: &Expression) -> Result<f64, Failure> {
    let degrees = context.value_as(expression, "taper angle", Dimension::ANGLE)?;
    if degrees.abs() >= MAX_COIL_TAPER_DEGREES {
        let shape = context.shape();
        return Err(context.error(CoilCause::TaperSteep {
            subject: format!("the {shape}"),
        }));
    }
    Ok(degrees.to_radians())
}

fn failure(
    context: &Context<'_>,
    feature: &Feature,
    spec: &CoilSpec,
    error: &CoilError,
) -> Failure {
    let name = feature.name.clone();
    match error {
        CoilError::Cancelled(_) => Failure::Cancelled,
        CoilError::TurnsOverlap { pitch, least } => context.error(CoilCause::TurnsOverlap {
            name,
            pitch: *pitch,
            least: *least,
        }),
        CoilError::ReachesAxis if spec.taper < 0.0 => {
            context.error(CoilCause::TaperDrawsIn { name })
        }
        CoilError::ReachesAxis => context.error(CoilCause::SectionReachesAxis { name }),
        CoilError::TooFewTurns { .. } | CoilError::TooManyTurns { .. } => {
            context.error(CoilCause::TurnsRangeOf { name })
        }
        CoilError::TaperTooSteep => context.error(CoilCause::TaperSteep { subject: name }),
        CoilError::TooLarge => context.error(CoilCause::TooFar { name }),
        CoilError::NonFinite
        | CoilError::NoRadius
        | CoilError::NoSection
        | CoilError::NoPitch
        | CoilError::Geometry(_)
        | CoilError::Unassembled
        | CoilError::Invalid(_) => {
            log::warn!("{name} could not be built: {error}");
            context.error(CoilCause::Unbuildable { name, helix: false })
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CoilCause {
    TurnsRange {
        shape: String,
        what: String,
    },
    HeightPitchTurns {
        shape: String,
        height: f64,
        pitch: f64,
        turns: f64,
    },
    TaperSteep {
        subject: String,
    },
    TurnsOverlap {
        name: String,
        pitch: f64,
        least: f64,
    },
    TaperDrawsIn {
        name: String,
    },
    HelixTaperDrawsIn {
        name: String,
    },
    SectionReachesAxis {
        name: String,
    },
    TurnsRangeOf {
        name: String,
    },
    TooFar {
        name: String,
    },
    Unbuildable {
        name: String,
        helix: bool,
    },
    NoDirection {
        name: String,
    },
    StartOnAxis {
        name: String,
    },
    OuterWithinInner {
        name: String,
    },
    OuterPitchTurns {
        name: String,
        width: f64,
        pitch: f64,
        turns: f64,
    },
    SpiralUnbuildable {
        name: String,
    },
    EndTurnsRange {
        name: String,
    },
    EndsTooLong {
        name: String,
        taken: f64,
        turns: f64,
    },
    EndsFillHeight {
        name: String,
        height: f64,
    },
    EndChangeRange {
        name: String,
    },
    ProgressionNeedsTurns {
        name: String,
        turns: f64,
    },
    FinalPitchFillsHeight {
        name: String,
        height: f64,
        final_pitch: f64,
    },
    PitchTableAlone {
        name: String,
        other: &'static str,
    },
    PitchTableOrder {
        name: String,
        turns: f64,
    },
    PitchTableBeyond {
        name: String,
        turns: f64,
        helix: f64,
    },
    PitchTableBeyondEnds {
        name: String,
        turns: f64,
        body: f64,
    },
    SpiralFinalPitchFillsWidth {
        name: String,
        width: f64,
        final_pitch: f64,
    },
    SpiralTableFillsWidth {
        name: String,
        width: f64,
    },
    PitchTableFillsHeight {
        name: String,
        height: f64,
    },
    PitchTableLong {
        name: String,
        rows: usize,
    },
}

impl CoilCause {
    pub(crate) fn reason(&self) -> String {
        match self {
            Self::TurnsRange { shape, .. } => {
                format!("The {shape} must make from {LEAST_COIL_TURNS} to {MAX_COIL_TURNS} turns.")
            }
            Self::HeightPitchTurns {
                shape,
                height,
                pitch,
                turns,
            } => format!(
                "A height of {height} mm at a pitch of {pitch} mm makes {turns:.2} turns; a \
                 {shape} makes from {LEAST_COIL_TURNS} to {MAX_COIL_TURNS}."
            ),
            Self::TaperSteep { subject } => {
                format!("The taper of {subject} is {MAX_COIL_TAPER_DEGREES}° or steeper.")
            }
            Self::TurnsOverlap { name, pitch, least } => format!(
                "The turns of {name} would touch: at a pitch of {pitch:.3} mm its section needs \
                 more than {least:.3} mm between turns."
            ),
            Self::TaperDrawsIn { name } => {
                format!("The taper draws {name} in until its section reaches the axis.")
            }
            Self::HelixTaperDrawsIn { name } => {
                format!("The taper draws {name} in until it reaches its axis.")
            }
            Self::SectionReachesAxis { name } => format!("The section of {name} reaches its axis."),
            Self::TurnsRangeOf { name } => {
                format!("{name} must make from {LEAST_COIL_TURNS} to {MAX_COIL_TURNS} turns.")
            }
            Self::TooFar { name } => format!("{name} reaches too far from the origin."),
            Self::Unbuildable { name, .. } => format!("{name} cannot be built with these sizes."),
            Self::NoDirection { name } => format!("The axis of {name} has no direction."),
            Self::StartOnAxis { name } => {
                format!("The start point of {name} lies on its axis, so it has no angle round it.")
            }
            Self::OuterWithinInner { name } => {
                format!("The outer diameter of {name} is not larger than its inner diameter.")
            }
            Self::OuterPitchTurns {
                name,
                width,
                pitch,
                turns,
            } => format!(
                "{name} grows {width} mm out to its outer diameter, which at a pitch of {pitch} mm \
                 makes {turns:.2} turns; a flat spiral makes from {LEAST_COIL_TURNS} to \
                 {MAX_COIL_TURNS}."
            ),
            Self::SpiralUnbuildable { name } => format!("{name} cannot be built with these sizes."),
            Self::EndTurnsRange { name } => format!(
                "The closed ends of {name} must make no turns or at least {LEAST_COIL_TURNS} of a \
                 turn each."
            ),
            Self::EndsTooLong { name, taken, turns } => format!(
                "The closed ends of {name} and the changes of pitch beside them take {taken} \
                 turns, leaving none or at least {LEAST_COIL_TURNS} between them, but it makes \
                 {turns:.2}."
            ),
            Self::EndsFillHeight { name, height } => format!(
                "The closed ends of {name} and the changes of pitch beside them rise {height} mm \
                 or more on their own, leaving nothing for the turns between them."
            ),
            Self::EndChangeRange { name } => format!(
                "The pitch of the closed ends of {name} must change over at least \
                 {LEAST_COIL_TURNS} of a turn."
            ),
            Self::ProgressionNeedsTurns { name, turns } => format!(
                "The closed ends of {name} and the changes of pitch beside them take all {turns:.2} \
                 turns it makes, leaving none for the pitch to change over from its pitch to its \
                 final pitch."
            ),
            Self::FinalPitchFillsHeight {
                name,
                height,
                final_pitch,
            } => format!(
                "A height of {height} mm with a final pitch of {final_pitch} mm leaves {name} no \
                 pitch above zero to start at."
            ),
            Self::PitchTableAlone { name, other } => format!(
                "{name} follows a table of pitches, which it cannot do with {other} as well."
            ),
            Self::PitchTableOrder { name, turns } => format!(
                "The row of the pitch table of {name} at turn {turns:.2} does not lie after the \
                 row before it, or the start."
            ),
            Self::PitchTableBeyond { name, turns, helix } => format!(
                "The pitch table of {name} has a row at turn {turns:.2}, but it makes only \
                 {helix:.2} turns."
            ),
            Self::PitchTableBeyondEnds { name, turns, body } => format!(
                "The pitch table of {name} counts turns from the start of the body between its \
                 closed ends, which makes {body:.2}, but has a row at turn {turns:.2}."
            ),
            Self::SpiralFinalPitchFillsWidth {
                name,
                width,
                final_pitch,
            } => format!(
                "Growing {width} mm out to its outer diameter with a final pitch of \
                 {final_pitch} mm leaves {name} no pitch above zero to start at."
            ),
            Self::SpiralTableFillsWidth { name, width } => format!(
                "Growing {width} mm out to its outer diameter with the rows of its pitch table \
                 leaves {name} no pitch above zero to start at."
            ),
            Self::PitchTableLong { name, rows } => format!(
                "The pitch table of {name} has {rows} rows, more than the {MAX_PITCH_ROWS} a \
                 helix takes."
            ),
            Self::PitchTableFillsHeight { name, height } => format!(
                "A height of {height} mm with the rows of its pitch table leaves {name} no pitch \
                 above zero to start at."
            ),
        }
    }

    pub(crate) fn remedy(&self) -> String {
        match self {
            Self::TurnsRange { what, .. } => {
                format!("Enter a number of {what} from {LEAST_COIL_TURNS} to {MAX_COIL_TURNS}.")
            }
            Self::HeightPitchTurns { .. } => "Change the height or the pitch.".to_owned(),
            Self::TaperSteep { .. } => format!(
                "Enter a taper angle between -{MAX_COIL_TAPER_DEGREES}° and \
                 {MAX_COIL_TAPER_DEGREES}°."
            ),
            Self::TurnsOverlap { .. } => {
                "Make the pitch larger (a greater height or fewer turns), or the section smaller."
                    .to_owned()
            }
            Self::TaperDrawsIn { .. } => {
                "Make the taper smaller, the coil shorter or its diameter larger.".to_owned()
            }
            Self::HelixTaperDrawsIn { .. } => {
                "Make the taper smaller, the helix shorter or its diameter larger.".to_owned()
            }
            Self::SectionReachesAxis { .. } => {
                "Make the diameter larger or the section smaller, or place the section outside \
                 the diameter."
                    .to_owned()
            }
            Self::TurnsRangeOf { .. } => "Change its turns, height or pitch.".to_owned(),
            Self::TooFar { .. } => {
                "Make it smaller or place its axis nearer the origin.".to_owned()
            }
            Self::Unbuildable { helix: true, .. } => {
                "Change its diameter, pitch or taper.".to_owned()
            }
            Self::Unbuildable { helix: false, .. } => {
                "Change its diameter, section, pitch or taper.".to_owned()
            }
            Self::NoDirection { .. } => "Choose another axis.".to_owned(),
            Self::StartOnAxis { .. } => {
                "Pick a point off the axis, or type a start angle instead.".to_owned()
            }
            Self::OuterWithinInner { .. } => {
                "Make the outer diameter larger than the inner diameter.".to_owned()
            }
            Self::OuterPitchTurns { .. } => "Change the outer diameter or the pitch.".to_owned(),
            Self::SpiralUnbuildable { .. } => {
                "Change its inner diameter, turns or pitch.".to_owned()
            }
            Self::EndTurnsRange { .. } => {
                format!("Enter 0 end turns, or {LEAST_COIL_TURNS} or more.")
            }
            Self::EndsTooLong { .. } => {
                "Make more turns or fewer end turns, or leave the ends open.".to_owned()
            }
            Self::EndsFillHeight { .. } => {
                "Make the height larger, or the end turns or end pitch smaller.".to_owned()
            }
            Self::EndChangeRange { .. } => {
                format!("Enter {LEAST_COIL_TURNS} or more turns for the change.")
            }
            Self::ProgressionNeedsTurns { .. } => {
                "Make more turns, fewer or shorter end turns, or give the final pitch the \
                 pitch's value."
                    .to_owned()
            }
            Self::FinalPitchFillsHeight { .. } => {
                "Make the height larger or the final pitch smaller.".to_owned()
            }
            Self::PitchTableAlone { .. } => {
                "Leave out the final pitch, or clear the table.".to_owned()
            }
            Self::PitchTableOrder { .. } => {
                "Put the rows in order of turn, each after the one before, the first after the \
                 start."
                    .to_owned()
            }
            Self::PitchTableBeyond { .. } => {
                "Move the row inside the turns it makes, or make more turns.".to_owned()
            }
            Self::PitchTableBeyondEnds { .. } => {
                "Move the row inside the turns between the closed ends, make more turns or \
                 shorten the ends."
                    .to_owned()
            }
            Self::SpiralFinalPitchFillsWidth { .. } => {
                "Make the outer diameter larger or the final pitch smaller.".to_owned()
            }
            Self::SpiralTableFillsWidth { .. } => {
                "Make the outer diameter larger or the pitches in the table smaller.".to_owned()
            }
            Self::PitchTableLong { .. } => {
                format!("Remove rows from the table, leaving {MAX_PITCH_ROWS} or fewer.")
            }
            Self::PitchTableFillsHeight { .. } => {
                "Make the height larger or the pitches in the table smaller.".to_owned()
            }
        }
    }
}

impl From<CoilCause> for Cause {
    fn from(cause: CoilCause) -> Self {
        Self::Coil(cause)
    }
}

pub(crate) fn evaluate(
    feature: &Feature,
    definition: &Coil,
    inputs: &Inputs<'_>,
    cancel: &CancelToken,
) -> Result<FeatureResult, Failure> {
    let context = Context::new(feature, inputs, COIL_SHAPE);
    let diameter = context.size(&definition.diameter, "diameter")?;
    let size = context.size(&definition.size, size_name(definition.shape))?;
    let span = span(&context, &definition.length)?;
    let taper = taper(&context, &definition.taper)?;
    let axis = context.resolver().axis(&definition.axis)?;
    let frame = definition.frame(axis).ok_or_else(|| {
        context.error(CoilCause::NoDirection {
            name: feature.name.clone(),
        })
    })?;
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let spec = CoilSpec {
        frame,
        radius: 0.5 * diameter,
        shape: definition.shape,
        size,
        placement: definition.placement,
        pitch: span.pitch,
        turns: span.turns,
        taper,
        hand: definition.turning.hand(),
    };
    let made = coil(&spec, feature.id().raw())
        .map_err(|error| failure(&context, feature, &spec, &error))?;
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    combined(&context, made, definition.operation)
}
