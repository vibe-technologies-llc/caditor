use std::{collections::BTreeSet, iter};

use caditor_kernel::{BooleanError, BooleanOperation, Solid, boolean};

use crate::{
    document::{Document, Feature, FeatureId},
    failure::Cause,
    recompute::{CancelToken, Evaluation, Failure, FeatureError, FeatureResult, FixTarget, Inputs},
    solid::SolidResult,
    trouble::{self, Adjust, Trouble, boolean_trouble},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CombineOperation {
    Join,
    Cut,
    Intersect,
}

impl CombineOperation {
    pub const ALL: [Self; 3] = [Self::Join, Self::Cut, Self::Intersect];

    pub fn verb(self) -> &'static str {
        match self {
            Self::Join => "Join",
            Self::Cut => "Cut",
            Self::Intersect => "Intersect",
        }
    }

    fn kernel(self) -> BooleanOperation {
        match self {
            Self::Join => BooleanOperation::Union,
            Self::Cut => BooleanOperation::Difference,
            Self::Intersect => BooleanOperation::Intersection,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Combine {
    pub body: FeatureId,
    pub tool: FeatureId,
    pub more_tools: Vec<FeatureId>,
    pub keep_tool: bool,
    pub operation: CombineOperation,
}

impl Combine {
    pub fn new(body: FeatureId, tool: FeatureId, operation: CombineOperation) -> Self {
        Self {
            body,
            tool,
            more_tools: Vec::new(),
            keep_tool: false,
            operation,
        }
    }

    pub fn tools(&self) -> impl Iterator<Item = FeatureId> + '_ {
        iter::once(self.tool).chain(self.more_tools.iter().copied())
    }

    pub fn features(&self) -> BTreeSet<FeatureId> {
        iter::once(self.body).chain(self.tools()).collect()
    }

    pub fn consumed(&self) -> Vec<FeatureId> {
        if self.keep_tool {
            Vec::new()
        } else {
            self.tools().collect()
        }
    }
}

struct Context<'a> {
    feature: &'a Feature,
    inputs: &'a Inputs<'a>,
}

impl Context<'_> {
    fn name(&self, id: FeatureId) -> String {
        self.inputs.document.body_label(id).unwrap_or_default()
    }

    fn error(&self, cause: CombineCause) -> Failure {
        Failure::from(FeatureError::new(cause).fixing(FixTarget::Feature(self.feature.id())))
    }
}

pub(crate) fn evaluate(
    feature: &Feature,
    definition: &Combine,
    inputs: &Inputs<'_>,
    cancel: &CancelToken,
) -> Result<FeatureResult, Failure> {
    let context = Context { feature, inputs };
    let mut seen = BTreeSet::from([definition.body]);
    for tool in definition.tools() {
        if tool == definition.body {
            return Err(context.error(CombineCause::WithItself));
        }
        if !seen.insert(tool) {
            return Err(context.error(CombineCause::RepeatedTool {
                tool: context.name(tool),
            }));
        }
    }
    let target = inputs
        .body(definition.body)
        .ok_or_else(|| inputs.missing_body(definition.body))?;
    let mut combined: Option<Solid> = None;
    for tool_id in definition.tools() {
        let tool = inputs
            .body(tool_id)
            .ok_or_else(|| inputs.missing_body(tool_id))?;
        if cancel.is_cancelled() {
            return Err(Failure::Cancelled);
        }
        let base = combined.as_ref().unwrap_or(target);
        let next = boolean(base, tool, definition.operation.kernel())
            .map_err(|error| failure(&context, definition, tool_id, [base, tool], &error))?;
        combined = Some(next);
    }
    let combined = combined.ok_or_else(|| inputs.missing_body(definition.body))?;
    Ok(FeatureResult::Solid(SolidResult::new(
        definition.body,
        combined,
    )))
}

fn failure(
    context: &Context<'_>,
    definition: &Combine,
    tool_id: FeatureId,
    operands: [&Solid; 2],
    error: &BooleanError,
) -> Failure {
    let target = context.name(definition.body);
    let tool = context.name(tool_id);
    let failure = match error {
        BooleanError::Cancelled(_) => Failure::Cancelled,
        BooleanError::Empty => context.error(CombineCause::Nothing {
            operation: definition.operation,
            target,
            tool,
        }),
        BooleanError::NonManifold(_) => context.error(CombineCause::Edgewise { target, tool }),
        BooleanError::Intersection { .. }
        | BooleanError::Split(_)
        | BooleanError::Ambiguous(_)
        | BooleanError::Open(_)
        | BooleanError::Invalid(_) => {
            log::warn!(
                "{} could not combine its bodies: {error}",
                context.feature.name
            );
            let trouble = boolean_trouble(
                context.inputs.document,
                operands,
                error,
                Adjust::MoveOrResizeABody,
            );
            context.error(CombineCause::Uncombined {
                target,
                tool,
                trouble,
            })
        }
    };
    failure.placed(trouble::place(error))
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CombineCause {
    WithItself,
    RepeatedTool {
        tool: String,
    },
    Nothing {
        operation: CombineOperation,
        target: String,
        tool: String,
    },
    Edgewise {
        target: String,
        tool: String,
    },
    Uncombined {
        target: String,
        tool: String,
        trouble: Trouble,
    },
}

impl CombineCause {
    pub(crate) fn reason(&self) -> String {
        match self {
            Self::WithItself => "A body cannot be combined with itself.".to_owned(),
            Self::RepeatedTool { tool } => format!("{tool} is chosen as a tool more than once."),
            Self::Nothing {
                operation: CombineOperation::Intersect,
                target,
                tool,
            } => format!("The bodies of {target} and {tool} do not overlap, so nothing is left."),
            Self::Nothing { target, tool, .. } => {
                format!("Cutting {tool} from {target} would leave nothing.")
            }
            Self::Edgewise { target, tool } => format!(
                "The result would have parts of {target} and {tool} that meet only along an edge."
            ),
            Self::Uncombined {
                target,
                tool,
                trouble,
            } => trouble.reason(format!(
                "The bodies of {target} and {tool} could not be combined."
            )),
        }
    }

    pub(crate) fn remedy(&self) -> String {
        match self {
            Self::WithItself => "Choose another body for the tool.".to_owned(),
            Self::RepeatedTool { .. } => "Choose each tool body once.".to_owned(),
            Self::Nothing { .. } => {
                "Move a body so part of the target stays, or choose another operation.".to_owned()
            }
            Self::Edgewise { .. } => "Move a body so they overlap more or stay clear.".to_owned(),
            Self::Uncombined { trouble, .. } => trouble.remedy(),
        }
    }
}

impl From<CombineCause> for Cause {
    fn from(cause: CombineCause) -> Self {
        Self::Combine(cause)
    }
}

impl Document {
    pub fn bodies_before(&self, feature: FeatureId) -> Vec<FeatureId> {
        let end = self.feature_index(feature).unwrap_or(0);
        self.bodies_at(end)
    }

    pub fn bodies_standing(&self) -> Vec<FeatureId> {
        self.bodies_at(self.bar_index())
    }

    pub fn bodies_before_in(&self, feature: FeatureId, evaluation: &Evaluation) -> Vec<FeatureId> {
        let end = self.feature_index(feature).unwrap_or(0);
        self.bodies_made(end, Some(evaluation))
    }

    pub fn bodies_standing_in(&self, evaluation: &Evaluation) -> Vec<FeatureId> {
        self.bodies_made(self.bar_index(), Some(evaluation))
    }

    fn bodies_at(&self, end: usize) -> Vec<FeatureId> {
        self.bodies_made(end, None)
    }

    fn bodies_made(&self, end: usize, evaluation: Option<&Evaluation>) -> Vec<FeatureId> {
        let mut bodies: Vec<FeatureId> = Vec::new();
        for earlier in self
            .features()
            .take(end)
            .filter(|earlier| !earlier.suppressed)
        {
            match evaluation {
                Some(evaluation) if earlier.separates_pieces() => {
                    bodies.extend(evaluation.pieces_made_by(earlier.id()));
                }
                _ if earlier.makes_body() => bodies.push(earlier.id()),
                _ => {}
            }
            for consumed in earlier.kind.consumed_bodies() {
                bodies.retain(|body| *body != consumed);
            }
        }
        bodies
    }
}
