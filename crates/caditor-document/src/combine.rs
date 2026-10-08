use std::{collections::BTreeSet, iter};

use caditor_kernel::{BooleanError, BooleanOperation, Solid, boolean};

use crate::{
    document::{Document, Feature, FeatureId},
    recompute::{CancelToken, Failure, FeatureError, FeatureResult, FixTarget, Inputs},
    solid::SolidResult,
    trouble::{self, boolean_trouble},
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
        self.inputs
            .document
            .feature(id)
            .map(|feature| feature.name.clone())
            .unwrap_or_default()
    }

    fn error(&self, reason: String, remedy: String) -> Failure {
        Failure::Error(Box::new(FeatureError {
            reason,
            remedy,
            fix: Some(FixTarget::Feature(self.feature.id())),
            constraints: Vec::new(),
            place: None,
        }))
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
            return Err(context.error(
                "A body cannot be combined with itself.".to_owned(),
                "Choose another body for the tool.".to_owned(),
            ));
        }
        if !seen.insert(tool) {
            return Err(context.error(
                format!("{} is chosen as a tool more than once.", context.name(tool)),
                "Choose each tool body once.".to_owned(),
            ));
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
        BooleanError::Empty => {
            let reason = match definition.operation {
                CombineOperation::Intersect => {
                    format!("The bodies of {target} and {tool} do not overlap, so nothing is left.")
                }
                CombineOperation::Cut | CombineOperation::Join => {
                    format!("Cutting {tool} from {target} would leave nothing.")
                }
            };
            context.error(
                reason,
                "Move a body so part of the target stays, or choose another operation.".to_owned(),
            )
        }
        BooleanError::NonManifold(_) => context.error(
            format!(
                "The result would have parts of {target} and {tool} that meet only along an edge."
            ),
            "Move a body so they overlap more or stay clear.".to_owned(),
        ),
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
                "Move or resize a body",
            );
            context.error(
                trouble.reason(format!(
                    "The bodies of {target} and {tool} could not be combined."
                )),
                trouble.remedy,
            )
        }
    };
    failure.placed(trouble::place(error))
}

impl Document {
    pub fn bodies_before(&self, feature: FeatureId) -> Vec<FeatureId> {
        let end = self.feature_index(feature).unwrap_or(0);
        self.bodies_at(end)
    }

    pub fn bodies_standing(&self) -> Vec<FeatureId> {
        self.bodies_at(self.bar_index())
    }

    fn bodies_at(&self, end: usize) -> Vec<FeatureId> {
        let mut bodies: Vec<FeatureId> = Vec::new();
        for earlier in self
            .features()
            .take(end)
            .filter(|earlier| !earlier.suppressed)
        {
            if earlier.makes_body() {
                bodies.push(earlier.id());
            }
            for consumed in earlier.kind.consumed_bodies() {
                bodies.retain(|body| *body != consumed);
            }
        }
        bodies
    }
}
