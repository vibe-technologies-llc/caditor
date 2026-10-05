use std::collections::BTreeSet;

use caditor_kernel::{BooleanError, BooleanOperation, boolean};

use crate::{
    document::{Document, Feature, FeatureId},
    recompute::{CancelToken, Failure, FeatureError, FeatureResult, FixTarget, Inputs},
    solid::SolidResult,
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
    pub operation: CombineOperation,
}

impl Combine {
    pub fn features(&self) -> BTreeSet<FeatureId> {
        BTreeSet::from([self.body, self.tool])
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
        Failure::Error(FeatureError {
            reason,
            remedy,
            fix: Some(FixTarget::Feature(self.feature.id())),
            constraints: Vec::new(),
        })
    }

    fn missing(&self, body: FeatureId) -> Failure {
        let name = self.name(body);
        Failure::Error(FeatureError {
            reason: format!("The body made by {name} has no shape."),
            remedy: format!("Fix {name} first."),
            fix: Some(FixTarget::Feature(body)),
            constraints: Vec::new(),
        })
    }
}

pub(crate) fn evaluate(
    feature: &Feature,
    definition: &Combine,
    inputs: &Inputs<'_>,
    cancel: &CancelToken,
) -> Result<FeatureResult, Failure> {
    let context = Context { feature, inputs };
    if definition.body == definition.tool {
        return Err(context.error(
            "A body cannot be combined with itself.".to_owned(),
            "Choose another body for the tool.".to_owned(),
        ));
    }
    let target = inputs
        .body(definition.body)
        .ok_or_else(|| context.missing(definition.body))?;
    let tool = inputs
        .body(definition.tool)
        .ok_or_else(|| context.missing(definition.tool))?;
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let combined = boolean(target, tool, definition.operation.kernel())
        .map_err(|error| failure(&context, definition, &error))?;
    Ok(FeatureResult::Solid(SolidResult::new(
        definition.body,
        combined,
    )))
}

fn failure(context: &Context<'_>, definition: &Combine, error: &BooleanError) -> Failure {
    let target = context.name(definition.body);
    let tool = context.name(definition.tool);
    match error {
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
        BooleanError::NonManifold => context.error(
            format!(
                "The result would have parts of {target} and {tool} that meet only along an edge."
            ),
            "Move a body so they overlap more or stay clear.".to_owned(),
        ),
        BooleanError::Intersection(_)
        | BooleanError::Split
        | BooleanError::Ambiguous
        | BooleanError::Open
        | BooleanError::Invalid(_) => {
            log::warn!(
                "{} could not combine its bodies: {error}",
                context.feature.name
            );
            context.error(
                format!("The bodies of {target} and {tool} could not be combined."),
                "Move or resize a body slightly; faces or edges that exactly touch can cause this."
                    .to_owned(),
            )
        }
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
