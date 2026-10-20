use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use caditor_expression::{Expression, ParameterId};
use caditor_kernel::{
    PathSweepError, PipeError, PipeSection, PipeShape, PipeStand, pipe, pipe_stand,
};

use crate::{
    document::{Document, Feature, FeatureId},
    primitive::{Context, combined},
    recompute::{CancelToken, Evaluation, Failure, FeatureResult, FixTarget, Inputs},
    solid::BodyOperation,
    sweep::{Path, SweepPath, failed, path_failure, path_of, spiral_of},
    sweep_cause::{Follower, Segments, SweepCause},
};

pub const PIPE_SHAPE: &str = "pipe";

pub fn shape_name(shape: PipeShape) -> &'static str {
    match shape {
        PipeShape::Round => "Round",
        PipeShape::Square => "Square",
        PipeShape::Triangle => "Triangle",
    }
}

pub fn size_name(shape: PipeShape) -> &'static str {
    match shape {
        PipeShape::Round => "diameter",
        PipeShape::Square => "width",
        PipeShape::Triangle => "side length",
    }
}

pub fn side_name(shape: PipeShape, entity: u64) -> &'static str {
    if entity > shape.curves() {
        "inside"
    } else {
        "outside"
    }
}

pub(crate) fn wall_limit_words(shape: PipeShape) -> &'static str {
    match shape {
        PipeShape::Round => "half its diameter",
        PipeShape::Square => "half its width",
        PipeShape::Triangle => "a third of the triangle's height",
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pipe {
    pub path: SweepPath,
    pub shape: PipeShape,
    pub size: Expression,
    pub wall: Option<Expression>,
    pub bend: Expression,
    pub operation: BodyOperation,
}

impl Pipe {
    pub fn new(path: SweepPath, size: Expression, bend: Expression) -> Self {
        Self {
            path,
            shape: PipeShape::Round,
            size,
            wall: None,
            bend,
            operation: BodyOperation::NewBody,
        }
    }

    pub fn expressions(&self) -> Vec<&Expression> {
        let mut expressions = vec![&self.size, &self.bend];
        expressions.extend(self.wall.as_ref());
        expressions
    }

    pub fn expressions_mut(&mut self) -> Vec<&mut Expression> {
        let mut expressions = vec![&mut self.size, &mut self.bend];
        expressions.extend(self.wall.as_mut());
        expressions
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
            .chain(self.path.features())
            .collect()
    }

    pub fn origin_features(&self) -> BTreeSet<FeatureId> {
        self.path.origin_features()
    }

    pub fn heap_size(&self) -> usize {
        self.path.heap_size()
            + self
                .expressions()
                .into_iter()
                .map(Expression::heap_size)
                .sum::<usize>()
    }
}

pub fn stand(
    document: &Document,
    evaluation: &Evaluation,
    feature: &Feature,
    definition: &Pipe,
    bend: f64,
) -> Option<PipeStand> {
    let features: BTreeMap<FeatureId, Arc<FeatureResult>> = definition
        .path
        .sketch()
        .into_iter()
        .chain(definition.path.helix())
        .filter_map(|source| Some((source, evaluation.feature(source)?.result.clone()?)))
        .collect();
    let bodies: BTreeMap<FeatureId, (FeatureId, Arc<FeatureResult>)> = definition
        .path
        .body()
        .and_then(|body| {
            let state = evaluation.body_state_seen_by(feature.id(), body)?;
            Some((body, (feature.id(), Arc::clone(state))))
        })
        .into_iter()
        .collect();
    let inputs = Inputs {
        document,
        parameters: &evaluation.parameters,
        features: &features,
        bodies: &bodies,
        previous: None,
    };
    let path = path_of(feature, &inputs, &definition.path).ok()?;
    pipe_stand(&path.segments, bend, path.up).ok()
}

fn failure(feature: &Feature, path: &Path, shape: PipeShape, error: &PipeError) -> Failure {
    let name = feature.name.clone();
    let own = FixTarget::Feature(feature.id());
    let at = |segments: &[usize]| Segments {
        labels: path.labels(segments),
        owner: path.owner.clone(),
    };
    match error {
        PipeError::WallTooThick => failed(SweepCause::WallTooThick { name, shape }, own),
        PipeError::BendTooLong { segments } => {
            failed(SweepCause::BendTooLong { at: at(segments) }, own)
        }
        PipeError::BendTooTight { segments } => failed(
            SweepCause::BendTooTight {
                at: at(segments),
                name,
            },
            own,
        ),
        PipeError::Path(PathSweepError::CornerTooSharp { segments }) => failed(
            SweepCause::CornerTooSharp {
                at: at(segments),
                name,
                follower: Follower::Pipe,
            },
            own,
        ),
        PipeError::Path(PathSweepError::Corner { segments }) => failed(
            SweepCause::Corner {
                at: at(segments),
                name,
                follower: Follower::Pipe,
            },
            own,
        ),
        PipeError::Path(error) => path_failure(feature, path, error),
        PipeError::InvalidSize
        | PipeError::InvalidWall
        | PipeError::InvalidBend
        | PipeError::Section(_) => {
            log::warn!("{name} could not be built: {error}");
            failed(SweepCause::PipeUnbuilt { name }, own)
        }
    }
}

fn clear_turns(
    feature: &Feature,
    inputs: &Inputs<'_>,
    path: &SweepPath,
    size: f64,
) -> Result<(), Failure> {
    let Some((spiral, spec)) = spiral_of(inputs, path) else {
        return Ok(());
    };
    if spec.turns_clear_of(size) {
        return Ok(());
    }
    Err(failed(
        SweepCause::SpiralTurnsTouch {
            name: feature.name.clone(),
            spiral,
            pitch: spec.pitch,
            least: spec.least_pitch_for(size),
        },
        FixTarget::Feature(feature.id()),
    ))
}

pub(crate) fn evaluate(
    feature: &Feature,
    definition: &Pipe,
    inputs: &Inputs<'_>,
    cancel: &CancelToken,
) -> Result<FeatureResult, Failure> {
    let context = Context::new(feature, inputs, PIPE_SHAPE);
    let shape = definition.shape;
    let size = context.size(&definition.size, size_name(shape))?;
    let wall = definition
        .wall
        .as_ref()
        .map(|wall| context.size(wall, "wall thickness"))
        .transpose()?;
    let bend = context.size_or_zero(&definition.bend, "bend radius")?;
    let path = path_of(feature, inputs, &definition.path)?;
    clear_turns(feature, inputs, &definition.path, size)?;
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let section = PipeSection { shape, size, wall };
    let made = pipe(&path.segments, &section, bend, path.up, feature.id().raw())
        .map_err(|error| failure(feature, &path, shape, &error))?;
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    combined(&context, made, definition.operation)
}
