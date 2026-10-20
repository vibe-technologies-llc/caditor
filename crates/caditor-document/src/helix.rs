use std::collections::BTreeSet;

use caditor_expression::{Dimension, Expression, ParameterId, Unit};
use caditor_geometry::{Plane, Point3, Ray};
use caditor_kernel::{
    HelixError, HelixSpec, HelixTurn, LEAST_COIL_TURNS, LINEAR_RESOLUTION, MAX_COIL_TURNS,
    SpiralError, SpiralSpec, helix, spiral,
};

use crate::{
    coil::{self, CoilCause, CoilLength, CoilSpan, CoilTurning, DEFAULT_PITCH, DEFAULT_TURNS},
    datum::{AxisReference, PointReference},
    document::{Feature, FeatureId},
    primitive::Context,
    recompute::{CancelToken, Failure, FeatureResult, Inputs},
};

pub const HELIX_SHAPE: &str = "helix";
pub const SPIRAL_SHAPE: &str = "flat spiral";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum HelixKind {
    #[default]
    Helix,
    FlatSpiral,
}

impl HelixKind {
    pub const ALL: [Self; 2] = [Self::Helix, Self::FlatSpiral];

    pub fn name(self) -> &'static str {
        match self {
            Self::Helix => "Helix",
            Self::FlatSpiral => "Flat spiral",
        }
    }

    pub fn shape(self) -> &'static str {
        match self {
            Self::Helix => HELIX_SHAPE,
            Self::FlatSpiral => SPIRAL_SHAPE,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Helix {
    pub kind: HelixKind,
    pub axis: AxisReference,
    pub reversed: bool,
    pub diameter: Expression,
    pub length: CoilLength,
    pub taper: Expression,
    pub turning: CoilTurning,
    pub start: Expression,
    pub start_point: Option<PointReference>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HelixResult {
    pub frame: Plane,
    pub turns: Vec<HelixTurn>,
    pub spiral: Option<SpiralSpec>,
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
            kind: HelixKind::Helix,
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
            start_point: None,
        }
    }

    pub fn flat_spiral(axis: AxisReference, inner_diameter: Expression) -> Self {
        Self {
            kind: HelixKind::FlatSpiral,
            ..Self::new(axis, inner_diameter)
        }
    }

    pub fn is_flat(&self) -> bool {
        self.kind == HelixKind::FlatSpiral
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
        let point = self.start_point.as_ref();
        self.axis
            .datum()
            .into_iter()
            .chain(self.axis.body())
            .chain(self.axis.sketch())
            .chain(self.axis.frame())
            .chain(point.and_then(PointReference::datum))
            .chain(point.and_then(PointReference::body))
            .chain(point.and_then(PointReference::sketch))
            .chain(point.and_then(PointReference::frame))
            .collect()
    }

    pub fn origin_features(&self) -> BTreeSet<FeatureId> {
        self.axis
            .origin_features()
            .into_iter()
            .chain(
                self.start_point
                    .iter()
                    .flat_map(PointReference::origin_features),
            )
            .collect()
    }

    pub fn heap_size(&self) -> usize {
        self.axis.heap_size()
            + self
                .start_point
                .as_ref()
                .map_or(0, PointReference::heap_size)
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
    let name = feature.name.clone();
    match error {
        HelixError::ReachesAxis => context.error(CoilCause::HelixTaperDrawsIn { name }),
        HelixError::TooFewTurns { .. } | HelixError::TooManyTurns { .. } => {
            context.error(CoilCause::TurnsRangeOf { name })
        }
        HelixError::TaperTooSteep => context.error(CoilCause::TaperSteep { subject: name }),
        HelixError::TooLarge => context.error(CoilCause::TooFar { name }),
        HelixError::NonFinite
        | HelixError::NoRadius
        | HelixError::NoPitch
        | HelixError::Geometry(_) => {
            log::warn!("{name} could not be built: {error}");
            context.error(CoilCause::Unbuildable { name, helix: true })
        }
    }
}

fn spiral_failure(context: &Context<'_>, feature: &Feature, error: &SpiralError) -> Failure {
    let name = feature.name.clone();
    match error {
        SpiralError::TooFewTurns { .. } | SpiralError::TooManyTurns { .. } => {
            context.error(CoilCause::TurnsRangeOf { name })
        }
        SpiralError::TooLarge => context.error(CoilCause::TooFar { name }),
        SpiralError::NonFinite
        | SpiralError::NoRadius
        | SpiralError::NoPitch
        | SpiralError::Unfitted
        | SpiralError::Geometry(_) => {
            log::warn!("{name} could not be built: {error}");
            context.error(CoilCause::SpiralUnbuildable { name })
        }
    }
}

fn outward(
    context: &Context<'_>,
    feature: &Feature,
    inner: f64,
    outer: &Expression,
) -> Result<f64, Failure> {
    let outer = context.size(outer, "outer diameter")?;
    if outer <= inner + 2.0 * LINEAR_RESOLUTION {
        return Err(context.error(CoilCause::OuterWithinInner {
            name: feature.name.clone(),
        }));
    }
    Ok(0.5 * (outer - inner))
}

fn spiral_span(
    context: &Context<'_>,
    feature: &Feature,
    inner: f64,
    length: &CoilLength,
) -> Result<CoilSpan, Failure> {
    match length {
        CoilLength::TurnsAndPitch { .. } => coil::span(context, length),
        CoilLength::HeightAndTurns { height, turns } => {
            let width = outward(context, feature, inner, height)?;
            let turns = coil::number(context, turns, "turns")?;
            Ok(CoilSpan {
                height: width,
                pitch: width / turns,
                turns,
            })
        }
        CoilLength::HeightAndPitch { height, pitch } => {
            let width = outward(context, feature, inner, height)?;
            let pitch = context.size(pitch, "pitch")?;
            let turns = width / pitch;
            if !(LEAST_COIL_TURNS..=MAX_COIL_TURNS).contains(&turns) {
                return Err(context.error(CoilCause::OuterPitchTurns {
                    name: feature.name.clone(),
                    width,
                    pitch,
                    turns,
                }));
            }
            Ok(CoilSpan {
                height: width,
                pitch,
                turns,
            })
        }
    }
}

pub(crate) fn evaluate(
    feature: &Feature,
    definition: &Helix,
    inputs: &Inputs<'_>,
    cancel: &CancelToken,
) -> Result<FeatureResult, Failure> {
    if definition.is_flat() {
        return evaluate_spiral(feature, definition, inputs, cancel);
    }
    let context = Context::new(feature, inputs, HELIX_SHAPE);
    let diameter = context.size(&definition.diameter, "diameter")?;
    let span = coil::span(&context, &definition.length)?;
    let taper = coil::taper(&context, &definition.taper)?;
    let frame = frame_of(&context, feature, definition)?;
    let start = start_angle(&context, feature, definition, &frame)?;
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
        start_angle: start,
    };
    let turns = helix(&spec).map_err(|error| failure(&context, feature, &error))?;
    Ok(FeatureResult::Helix(HelixResult {
        frame,
        turns,
        spiral: None,
    }))
}

fn evaluate_spiral(
    feature: &Feature,
    definition: &Helix,
    inputs: &Inputs<'_>,
    cancel: &CancelToken,
) -> Result<FeatureResult, Failure> {
    let context = Context::new(feature, inputs, SPIRAL_SHAPE);
    let diameter = context.size(&definition.diameter, "inner diameter")?;
    let span = spiral_span(&context, feature, diameter, &definition.length)?;
    let frame = frame_of(&context, feature, definition)?;
    let start = start_angle(&context, feature, definition, &frame)?;
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let spec = SpiralSpec {
        frame,
        radius: 0.5 * diameter,
        pitch: span.pitch,
        turns: span.turns,
        hand: definition.turning.hand(),
        start_angle: start,
    };
    let turns = spiral(&spec).map_err(|error| spiral_failure(&context, feature, &error))?;
    Ok(FeatureResult::Helix(HelixResult {
        frame,
        turns,
        spiral: Some(spec),
    }))
}

fn frame_of(
    context: &Context<'_>,
    feature: &Feature,
    definition: &Helix,
) -> Result<Plane, Failure> {
    let axis = context.resolver().axis(&definition.axis)?;
    definition.frame(axis).ok_or_else(|| {
        context.error(CoilCause::NoDirection {
            name: feature.name.clone(),
        })
    })
}

fn start_angle(
    context: &Context<'_>,
    feature: &Feature,
    definition: &Helix,
    frame: &Plane,
) -> Result<f64, Failure> {
    let Some(reference) = &definition.start_point else {
        let typed = context.value_as(&definition.start, "start angle", Dimension::ANGLE)?;
        return Ok(typed.to_radians());
    };
    let point = context.resolver().point(reference)?;
    let around = frame.to_local(point);
    if around.length() <= LINEAR_RESOLUTION {
        return Err(context.error(CoilCause::StartOnAxis {
            name: feature.name.clone(),
        }));
    }
    Ok(around.y.atan2(around.x))
}
