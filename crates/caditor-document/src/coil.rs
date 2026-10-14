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

    pub(crate) fn hand(self) -> Handedness {
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

fn number(context: &Context<'_>, expression: &Expression, what: &str) -> Result<f64, Failure> {
    let value = context.value_as(expression, what, Dimension::NONE)?;
    if !(LEAST_COIL_TURNS..=MAX_COIL_TURNS).contains(&value) {
        let shape = context.shape();
        return Err(context.error(
            format!("The {shape} must make from {LEAST_COIL_TURNS} to {MAX_COIL_TURNS} turns."),
            format!("Enter a number of {what} from {LEAST_COIL_TURNS} to {MAX_COIL_TURNS}."),
        ));
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
                return Err(context.error(
                    format!(
                        "A height of {height} mm at a pitch of {pitch} mm makes {turns:.2} turns; \
                         a {shape} makes from {LEAST_COIL_TURNS} to {MAX_COIL_TURNS}."
                    ),
                    "Change the height or the pitch.".to_owned(),
                ));
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
        return Err(context.error(
            format!("The taper of the {shape} is {MAX_COIL_TAPER_DEGREES}° or steeper."),
            format!(
                "Enter a taper angle between -{MAX_COIL_TAPER_DEGREES}° and \
                 {MAX_COIL_TAPER_DEGREES}°."
            ),
        ));
    }
    Ok(degrees.to_radians())
}

fn failure(
    context: &Context<'_>,
    feature: &Feature,
    spec: &CoilSpec,
    error: &CoilError,
) -> Failure {
    let name = &feature.name;
    match error {
        CoilError::Cancelled(_) => Failure::Cancelled,
        CoilError::TurnsOverlap { pitch, least } => context.error(
            format!(
                "The turns of {name} would touch: at a pitch of {pitch:.3} mm its section needs \
                 more than {least:.3} mm between turns."
            ),
            "Make the pitch larger (a greater height or fewer turns), or the section smaller."
                .to_owned(),
        ),
        CoilError::ReachesAxis if spec.taper < 0.0 => context.error(
            format!("The taper draws {name} in until its section reaches the axis."),
            "Make the taper smaller, the coil shorter or its diameter larger.".to_owned(),
        ),
        CoilError::ReachesAxis => context.error(
            format!("The section of {name} reaches its axis."),
            "Make the diameter larger or the section smaller, or place the section outside the \
             diameter."
                .to_owned(),
        ),
        CoilError::TooFewTurns { .. } | CoilError::TooManyTurns { .. } => context.error(
            format!("{name} must make from {LEAST_COIL_TURNS} to {MAX_COIL_TURNS} turns."),
            "Change its turns, height or pitch.".to_owned(),
        ),
        CoilError::TaperTooSteep => context.error(
            format!("The taper of {name} is {MAX_COIL_TAPER_DEGREES}° or steeper."),
            format!(
                "Enter a taper angle between -{MAX_COIL_TAPER_DEGREES}° and \
                 {MAX_COIL_TAPER_DEGREES}°."
            ),
        ),
        CoilError::TooLarge => context.error(
            format!("{name} reaches too far from the origin."),
            "Make it smaller or place its axis nearer the origin.".to_owned(),
        ),
        CoilError::NonFinite
        | CoilError::NoRadius
        | CoilError::NoSection
        | CoilError::NoPitch
        | CoilError::Geometry(_)
        | CoilError::Unassembled
        | CoilError::Invalid(_) => {
            log::warn!("{name} could not be built: {error}");
            context.error(
                format!("{name} cannot be built with these sizes."),
                "Change its diameter, section, pitch or taper.".to_owned(),
            )
        }
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
        context.error(
            format!("The axis of {} has no direction.", feature.name),
            "Choose another axis.".to_owned(),
        )
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
