use std::collections::BTreeSet;

use caditor_expression::{Dimension, Expression, ParameterId, Unit};
use caditor_geometry::{Plane, Point3, Ray};
use caditor_kernel::{HelixError, HelixSpec, HelixTurn, MAX_COIL_TAPER_DEGREES, helix};

use crate::{
    coil::{self, CoilLength, CoilTurning, DEFAULT_PITCH, DEFAULT_TURNS},
    datum::AxisReference,
    document::{Feature, FeatureId},
    primitive::Context,
    recompute::{CancelToken, Failure, FeatureResult, Inputs},
};

pub const HELIX_SHAPE: &str = "helix";

#[derive(Debug, Clone, PartialEq)]
pub struct Helix {
    pub axis: AxisReference,
    pub reversed: bool,
    pub diameter: Expression,
    pub length: CoilLength,
    pub taper: Expression,
    pub turning: CoilTurning,
    pub start: Expression,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HelixResult {
    pub frame: Plane,
    pub turns: Vec<HelixTurn>,
}

impl HelixResult {
    pub fn start(&self) -> Option<Point3> {
        let first = self.turns.first()?;
        Some(first.curve().point(first.range().start()))
    }

    pub fn end(&self) -> Option<Point3> {
        let last = self.turns.last()?;
        Some(last.curve().point(last.range().end()))
    }
}

impl Helix {
    pub fn new(axis: AxisReference, diameter: Expression) -> Self {
        Self {
            axis,
            reversed: false,
            diameter,
            length: CoilLength::TurnsAndPitch {
                turns: Expression::number(DEFAULT_TURNS),
                pitch: Expression::measure(DEFAULT_PITCH, Unit::Millimetre),
            },
            taper: Expression::measure(0.0, Unit::Degree),
            turning: CoilTurning::CounterClockwise,
            start: Expression::measure(0.0, Unit::Degree),
        }
    }

    pub fn expressions(&self) -> Vec<&Expression> {
        let mut expressions = vec![&self.diameter, &self.taper, &self.start];
        expressions.extend(self.length.expressions());
        expressions
    }

    pub fn expressions_mut(&mut self) -> Vec<&mut Expression> {
        let mut expressions = vec![&mut self.diameter, &mut self.taper, &mut self.start];
        expressions.extend(self.length.expressions_mut());
        expressions
    }

    pub fn lengths_mut(&mut self) -> Vec<&mut Expression> {
        let mut lengths = vec![&mut self.diameter];
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
        self.axis
            .datum()
            .into_iter()
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

fn failure(context: &Context<'_>, feature: &Feature, error: &HelixError) -> Failure {
    let name = &feature.name;
    match error {
        HelixError::ReachesAxis => context.error(
            format!("The taper draws {name} in until it reaches its axis."),
            "Make the taper smaller, the helix shorter or its diameter larger.".to_owned(),
        ),
        HelixError::TooFewTurns { .. } | HelixError::TooManyTurns { .. } => context.error(
            format!(
                "{name} must make from {} to {} turns.",
                caditor_kernel::LEAST_COIL_TURNS,
                caditor_kernel::MAX_COIL_TURNS
            ),
            "Change its turns, height or pitch.".to_owned(),
        ),
        HelixError::TaperTooSteep => context.error(
            format!("The taper of {name} is {MAX_COIL_TAPER_DEGREES}° or steeper."),
            format!(
                "Enter a taper angle between -{MAX_COIL_TAPER_DEGREES}° and \
                 {MAX_COIL_TAPER_DEGREES}°."
            ),
        ),
        HelixError::TooLarge => context.error(
            format!("{name} reaches too far from the origin."),
            "Make it smaller or place its axis nearer the origin.".to_owned(),
        ),
        HelixError::NonFinite
        | HelixError::NoRadius
        | HelixError::NoPitch
        | HelixError::Geometry(_) => {
            log::warn!("{name} could not be built: {error}");
            context.error(
                format!("{name} cannot be built with these sizes."),
                "Change its diameter, pitch or taper.".to_owned(),
            )
        }
    }
}

pub(crate) fn evaluate(
    feature: &Feature,
    definition: &Helix,
    inputs: &Inputs<'_>,
    cancel: &CancelToken,
) -> Result<FeatureResult, Failure> {
    let context = Context::new(feature, inputs, HELIX_SHAPE);
    let diameter = context.size(&definition.diameter, "diameter")?;
    let span = coil::span(&context, &definition.length)?;
    let taper = coil::taper(&context, &definition.taper)?;
    let start = context.value_as(&definition.start, "start angle", Dimension::ANGLE)?;
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
    let spec = HelixSpec {
        frame,
        radius: 0.5 * diameter,
        pitch: span.pitch,
        turns: span.turns,
        taper,
        hand: definition.turning.hand(),
        start_angle: start.to_radians(),
    };
    let turns = helix(&spec).map_err(|error| failure(&context, feature, &error))?;
    Ok(FeatureResult::Helix(HelixResult { frame, turns }))
}
