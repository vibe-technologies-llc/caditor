use std::collections::BTreeSet;

use caditor_expression::{Dimension, EvalError, Expression, ParameterId};
use caditor_kernel::{BlendError, BlendShape, EdgeId, EdgeNaming, EdgeReference, Solid, blend};

use crate::{
    describe::describe_edge,
    document::{Feature, FeatureId},
    origins,
    pieces::{Resolution, Unresolved, pieces_of_one_edge, tally},
    recompute::{CancelToken, Failure, FeatureError, FeatureResult, FixTarget, Inputs},
    solid::SolidResult,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BlendKind {
    Fillet,
    Chamfer,
}

impl BlendKind {
    pub fn title(self) -> &'static str {
        match self {
            Self::Fillet => "Fillet",
            Self::Chamfer => "Chamfer",
        }
    }

    pub fn noun(self) -> &'static str {
        match self {
            Self::Fillet => "fillet",
            Self::Chamfer => "chamfer",
        }
    }

    pub fn size_name(self) -> &'static str {
        match self {
            Self::Fillet => "radius",
            Self::Chamfer => "distance",
        }
    }

    fn shape(self, size: f64) -> BlendShape {
        match self {
            Self::Fillet => BlendShape::Fillet { radius: size },
            Self::Chamfer => BlendShape::Chamfer { distance: size },
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Blend {
    pub kind: BlendKind,
    pub body: FeatureId,
    pub edges: Vec<EdgeReference>,
    pub size: Expression,
}

impl Blend {
    pub fn parameters(&self) -> BTreeSet<ParameterId> {
        self.size.parameters().into_iter().collect()
    }

    pub fn uses_parameter(&self, parameter: ParameterId) -> bool {
        self.size.uses(parameter)
    }

    pub fn features(&self) -> BTreeSet<FeatureId> {
        BTreeSet::from([self.body])
    }

    pub fn origin_features(&self) -> BTreeSet<FeatureId> {
        self.edges.iter().flat_map(origins::of_edge).collect()
    }

    pub fn resolutions(&self, solid: &Solid) -> Vec<Resolution<EdgeId>> {
        let naming = EdgeNaming::new(solid);
        self.edges
            .iter()
            .map(|reference| {
                Resolution::of(reference.resolve_in(&naming), |pieces| {
                    pieces_of_one_edge(solid, pieces)
                })
            })
            .collect()
    }

    pub fn resolve(&self, solid: &Solid) -> Result<Vec<EdgeId>, Unresolved> {
        tally(self.resolutions(solid))
    }
}

struct Context<'a> {
    feature: &'a Feature,
    definition: &'a Blend,
    inputs: &'a Inputs<'a>,
    body_name: String,
}

impl Context<'_> {
    fn error(&self, reason: String, remedy: String) -> Failure {
        Failure::Error(FeatureError {
            reason,
            remedy,
            fix: Some(FixTarget::Feature(self.feature.id())),
            constraints: Vec::new(),
        })
    }

    fn size(&self) -> Result<f64, Failure> {
        let what = self.definition.kind.size_name();
        let value = self
            .definition
            .size
            .evaluate_as(Dimension::LENGTH, &|id| self.inputs.parameters.value(id))
            .map_err(|error| {
                let (remedy, fix) = match &error {
                    EvalError::ParameterFailed { id, name } => (
                        format!("Fix {name} under Parameters, or edit the {what}."),
                        FixTarget::Parameter(*id),
                    ),
                    EvalError::WrongKind { .. } => (
                        format!("Edit the {what} so it gives a length, such as 2 mm."),
                        FixTarget::Feature(self.feature.id()),
                    ),
                    _ => (
                        format!("Edit the {what} or the parameters it uses."),
                        FixTarget::Feature(self.feature.id()),
                    ),
                };
                Failure::Error(FeatureError {
                    reason: format!("The {what} cannot be evaluated: {error}."),
                    remedy,
                    fix: Some(fix),
                    constraints: Vec::new(),
                })
            })?;
        if value > 0.0 {
            Ok(value)
        } else {
            Err(self.error(
                format!("The {what} must be more than zero."),
                format!("Enter a {what} above zero."),
            ))
        }
    }

    fn failure(&self, solid: &Solid, error: &BlendError) -> Failure {
        let kind = self.definition.kind;
        let noun = kind.noun();
        let what = kind.size_name();
        let edge = error.edge().map_or_else(
            || "an edge".to_owned(),
            |edge| describe_edge(self.inputs.document, solid, edge),
        );
        match error {
            BlendError::Cancelled(_) => Failure::Cancelled,
            BlendError::InvalidSize => self.error(
                format!("The {what} must be more than 0.000001 mm."),
                format!("Enter a larger {what}."),
            ),
            BlendError::NoEdges => self.error(
                "No edge is chosen.".to_owned(),
                format!(
                    "Choose at least one edge of the body of {}.",
                    self.body_name
                ),
            ),
            BlendError::MissingEdge(_) => self.error(
                format!(
                    "A chosen edge is no longer part of the body of {}.",
                    self.body_name
                ),
                "Choose the edges again, or undo the change that removed it.".to_owned(),
            ),
            BlendError::Unsupported(_) => self.error(
                format!(
                    "A {noun} cannot follow {edge}: only straight edges along flat or cylindrical \
                     faces, and circular edges around an axis, can be blended."
                ),
                "Leave this edge out.".to_owned(),
            ),
            BlendError::Smooth(_) => self.error(
                format!("The faces meet smoothly at {edge}, so there is no corner to blend."),
                "Leave this edge out.".to_owned(),
            ),
            BlendError::TooLarge(_) => self.error(
                format!("The {what} is too large for the faces next to {edge}."),
                format!("Enter a smaller {what}, or leave this edge out."),
            ),
            BlendError::WrapsAround(_) => self.error(
                format!(
                    "The {noun} of {edge} would run into itself: the edge is almost a full circle \
                     and its ends need room beyond it."
                ),
                format!(
                    "Also choose the edges that continue from it, enter a smaller {what}, or \
                     leave it out."
                ),
            ),
            BlendError::UnsupportedEnd { .. } => self.error(
                format!("The {noun} cannot be closed off where {edge} ends."),
                "Also choose the edges that continue from it, or leave it out.".to_owned(),
            ),
            BlendError::Lost(_) => self.error(
                format!(
                    "The {noun} of {edge} could not be finished after the inner corners next to \
                     it were filled."
                ),
                "Blend the inner and the outer edges in separate features.".to_owned(),
            ),
            BlendError::AfterFill(inner) => {
                log::warn!("{} could not be built: {inner}", self.feature.name);
                self.error(
                    format!(
                        "The {noun} could not be finished after the inner corners of the body of \
                         {} were filled.",
                        self.body_name
                    ),
                    "Blend the inner and the outer edges in separate features.".to_owned(),
                )
            }
            BlendError::Profile { .. } | BlendError::Sweep { .. } | BlendError::Boolean { .. } => {
                log::warn!("{} could not be built: {error}", self.feature.name);
                match error.edge() {
                    Some(_) => self.error(
                        format!("The {noun} of {edge} could not be built."),
                        format!("Change the {what} slightly, or leave this edge out."),
                    ),
                    None => self.error(
                        format!(
                            "The {noun} could not be built on the body of {}.",
                            self.body_name
                        ),
                        format!("Change the {what} slightly, or choose fewer edges."),
                    ),
                }
            }
        }
    }
}

pub(crate) fn evaluate(
    feature: &Feature,
    definition: &Blend,
    inputs: &Inputs<'_>,
    cancel: &CancelToken,
) -> Result<FeatureResult, Failure> {
    let body_name = inputs
        .document
        .feature(definition.body)
        .map(|body| body.name.clone())
        .unwrap_or_default();
    let context = Context {
        feature,
        definition,
        inputs,
        body_name,
    };
    let size = context.size()?;
    let Some(solid) = inputs.body(definition.body) else {
        return Err(Failure::Error(FeatureError {
            reason: format!("The body made by {} has no shape.", context.body_name),
            remedy: format!("Fix {} first.", context.body_name),
            fix: Some(FixTarget::Feature(definition.body)),
            constraints: Vec::new(),
        }));
    };
    let edges = definition.resolve(solid).map_err(|unresolved| {
        let body = &context.body_name;
        let reason = match unresolved {
            Unresolved::Missing(1) => {
                format!("A chosen edge is no longer part of the body of {body}.")
            }
            Unresolved::Missing(missing) => {
                format!("{missing} chosen edges are no longer part of the body of {body}.")
            }
            Unresolved::Unrelated(1) => {
                format!("A chosen edge now matches several separate edges of the body of {body}.")
            }
            Unresolved::Unrelated(unrelated) => format!(
                "{unrelated} chosen edges now match several separate edges of the body of {body}."
            ),
        };
        context.error(
            reason,
            "Choose the edges again, or undo the change that removed them.".to_owned(),
        )
    })?;
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let result = blend(
        solid,
        &edges,
        definition.kind.shape(size),
        feature.id().raw(),
    )
    .map_err(|error| context.failure(solid, &error))?;
    Ok(FeatureResult::Solid(SolidResult::new(
        definition.body,
        result,
    )))
}
