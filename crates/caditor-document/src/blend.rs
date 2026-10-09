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
}

#[derive(Debug, Clone, Default, PartialEq)]
pub enum ChamferForm {
    #[default]
    Equal,
    TwoDistances {
        second: Expression,
    },
    DistanceAngle {
        angle: Expression,
    },
}

impl ChamferForm {
    pub fn title(&self) -> &'static str {
        match self {
            Self::Equal => "Equal",
            Self::TwoDistances { .. } => "Two distances",
            Self::DistanceAngle { .. } => "Distance and angle",
        }
    }

    pub fn is_equal(&self) -> bool {
        matches!(self, Self::Equal)
    }

    fn expression(&self) -> Option<&Expression> {
        match self {
            Self::Equal => None,
            Self::TwoDistances { second } => Some(second),
            Self::DistanceAngle { angle } => Some(angle),
        }
    }

    pub fn expression_mut(&mut self) -> Option<&mut Expression> {
        match self {
            Self::Equal => None,
            Self::TwoDistances { second } => Some(second),
            Self::DistanceAngle { angle } => Some(angle),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Blend {
    pub kind: BlendKind,
    pub body: FeatureId,
    pub edges: Vec<EdgeReference>,
    pub size: Expression,
    pub form: ChamferForm,
    pub flipped: bool,
}

impl Blend {
    pub fn chamfer_form(&self) -> &ChamferForm {
        match self.kind {
            BlendKind::Fillet => &ChamferForm::Equal,
            BlendKind::Chamfer => &self.form,
        }
    }

    pub fn expressions(&self) -> impl Iterator<Item = &Expression> {
        std::iter::once(&self.size).chain(self.chamfer_form().expression())
    }

    pub fn expressions_mut(&mut self) -> Vec<&mut Expression> {
        let form = match self.kind {
            BlendKind::Fillet => None,
            BlendKind::Chamfer => self.form.expression_mut(),
        };
        std::iter::once(&mut self.size).chain(form).collect()
    }

    pub fn heap_size(&self) -> usize {
        size_of_val(self.edges.as_slice())
            + self.size.heap_size()
            + self.form.expression().map_or(0, Expression::heap_size)
    }

    pub fn parameters(&self) -> BTreeSet<ParameterId> {
        self.expressions()
            .flat_map(Expression::parameters)
            .collect()
    }

    pub fn uses_parameter(&self, parameter: ParameterId) -> bool {
        self.expressions()
            .any(|expression| expression.uses(parameter))
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
        Failure::Error(Box::new(FeatureError {
            reason,
            remedy,
            fix: Some(FixTarget::Feature(self.feature.id())),
            constraints: Vec::new(),
            place: None,
        }))
    }

    fn value(
        &self,
        expression: &Expression,
        dimension: Dimension,
        what: &str,
    ) -> Result<f64, Failure> {
        let example = if dimension == Dimension::ANGLE {
            "an angle, such as 45 deg"
        } else {
            "a length, such as 2 mm"
        };
        let value = expression
            .evaluate_as(dimension, &|id| self.inputs.parameters.value(id))
            .map_err(|error| {
                let (remedy, fix) = match &error {
                    EvalError::ParameterFailed { id, name } => (
                        format!("Fix {name} under Parameters, or edit the {what}."),
                        FixTarget::Parameter(*id),
                    ),
                    EvalError::WrongKind { .. } => (
                        format!("Edit the {what} so it gives {example}."),
                        FixTarget::Feature(self.feature.id()),
                    ),
                    _ => (
                        format!("Edit the {what} or the parameters it uses."),
                        FixTarget::Feature(self.feature.id()),
                    ),
                };
                Failure::Error(Box::new(FeatureError {
                    reason: format!("The {what} cannot be evaluated: {error}."),
                    remedy,
                    fix: Some(fix),
                    constraints: Vec::new(),
                    place: None,
                }))
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

    fn shape(&self) -> Result<BlendShape, Failure> {
        let definition = self.definition;
        let size = self.value(
            &definition.size,
            Dimension::LENGTH,
            definition.kind.size_name(),
        )?;
        let flipped = definition.flipped;
        Ok(match (definition.kind, definition.chamfer_form()) {
            (BlendKind::Fillet, _) => BlendShape::Fillet { radius: size },
            (BlendKind::Chamfer, ChamferForm::Equal) => BlendShape::Chamfer { distance: size },
            (BlendKind::Chamfer, ChamferForm::TwoDistances { second }) => {
                BlendShape::TwoDistanceChamfer {
                    first: size,
                    second: self.value(second, Dimension::LENGTH, "second distance")?,
                    flipped,
                }
            }
            (BlendKind::Chamfer, ChamferForm::DistanceAngle { angle }) => {
                let degrees = self.value(angle, Dimension::ANGLE, "angle")?;
                if degrees >= 180.0 {
                    return Err(self.error(
                        "The angle must be less than 180 degrees.".to_owned(),
                        "Enter an angle between 0 and 180 degrees, such as 45 deg.".to_owned(),
                    ));
                }
                BlendShape::AngledChamfer {
                    distance: size,
                    angle: degrees.to_radians(),
                    flipped,
                }
            }
        })
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
            BlendError::InvalidAngle => self.error(
                "The angle must be between 0 and 180 degrees.".to_owned(),
                "Enter an angle between 0 and 180 degrees, such as 45 deg.".to_owned(),
            ),
            BlendError::AngleMisses(_) => self.error(
                format!("At this angle the cut never meets the other face next to {edge}."),
                "Enter a smaller angle, flip the chamfer to measure from the other face, or \
                 leave this edge out."
                    .to_owned(),
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
    let shape = context.shape()?;
    let Some(solid) = inputs.body(definition.body) else {
        return Err(inputs.missing_body(definition.body));
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
    let result = blend(solid, &edges, shape, feature.id().raw())
        .map_err(|error| context.failure(solid, &error))?;
    Ok(FeatureResult::Solid(SolidResult::new(
        definition.body,
        result,
    )))
}
