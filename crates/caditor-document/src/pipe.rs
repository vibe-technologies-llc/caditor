use std::collections::BTreeSet;

use caditor_expression::{Expression, ParameterId};
use caditor_kernel::{PathSweepError, PipeError, PipeSection, PipeShape, pipe};

use crate::{
    document::{Feature, FeatureId},
    primitive::{Context, combined},
    recompute::{CancelToken, Failure, FeatureResult, FixTarget, Inputs},
    solid::BodyOperation,
    sweep::{Path, SweepPath, failed, path_failure, path_of},
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

fn wall_limit_words(shape: PipeShape) -> &'static str {
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
            .chain(self.path.sketch())
            .chain(self.path.body())
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

fn failure(feature: &Feature, path: &Path, shape: PipeShape, error: &PipeError) -> Failure {
    let name = &feature.name;
    let owner = &path.owner;
    let own = FixTarget::Feature(feature.id());
    match error {
        PipeError::WallTooThick => failed(
            format!(
                "The wall of {name} is too thick: it must be thinner than {}.",
                wall_limit_words(shape)
            ),
            "Make the wall thinner or the pipe larger.".to_owned(),
            own,
        ),
        PipeError::BendTooLong { segments } => failed(
            format!(
                "Rounding the corner between {}{owner} to the bend radius needs more than the \
                 length of the lines there.",
                path.labels(segments)
            ),
            "Make the bend radius smaller, or lengthen the lines at the corner.".to_owned(),
            own,
        ),
        PipeError::BendTooTight { segments } => failed(
            format!(
                "The bend between {}{owner} is tighter than {name}: its section would reach past \
                 the centre the bend turns around.",
                path.labels(segments)
            ),
            "Make the bend radius larger or the pipe smaller.".to_owned(),
            own,
        ),
        PipeError::Path(PathSweepError::CornerTooSharp { segments }) => failed(
            format!(
                "The corner between {}{owner} is too sharp for {name} to turn.",
                path.labels(segments)
            ),
            "Open the corner up, lengthen the lines at it, or give the pipe a bend radius."
                .to_owned(),
            own,
        ),
        PipeError::Path(error) => path_failure(feature, path, error),
        PipeError::InvalidSize
        | PipeError::InvalidWall
        | PipeError::InvalidBend
        | PipeError::Section(_) => {
            log::warn!("{name} could not be built: {error}");
            failed(
                format!("{name} cannot be built with these sizes."),
                "Change its size, wall thickness or bend radius.".to_owned(),
                own,
            )
        }
    }
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
