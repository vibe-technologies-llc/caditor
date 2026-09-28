use std::collections::BTreeSet;

use caditor_expression::{Dimension, EvalError, Expression, ParameterId};
use caditor_kernel::{FaceId, FaceReference, ReferenceError, ShellError, Solid, shell};

use crate::{
    describe::{describe_edge, describe_origin},
    document::{Feature, FeatureId},
    recompute::{CancelToken, Failure, FeatureError, FeatureResult, FixTarget, Inputs},
    solid::SolidResult,
};

#[derive(Debug, Clone, PartialEq)]
pub struct Shell {
    pub body: FeatureId,
    pub open: Vec<FaceReference>,
    pub thickness: Expression,
}

impl Shell {
    pub fn parameters(&self) -> BTreeSet<ParameterId> {
        self.thickness.parameters().into_iter().collect()
    }

    pub fn uses_parameter(&self, parameter: ParameterId) -> bool {
        self.thickness.uses(parameter)
    }

    pub fn features(&self) -> BTreeSet<FeatureId> {
        BTreeSet::from([self.body])
    }

    pub fn resolve(&self, solid: &Solid) -> Result<Vec<FaceId>, usize> {
        let mut found = Vec::with_capacity(self.open.len());
        let mut missing = 0;
        for reference in &self.open {
            match reference.resolve(solid) {
                Ok(face) => found.push(face),
                Err(ReferenceError::Ambiguous(pieces)) => found.extend(pieces),
                Err(ReferenceError::Missing) => missing += 1,
            }
        }
        if missing > 0 {
            return Err(missing);
        }
        found.sort_unstable();
        found.dedup();
        Ok(found)
    }
}

struct Context<'a> {
    feature: &'a Feature,
    definition: &'a Shell,
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

    fn thickness(&self) -> Result<f64, Failure> {
        let value = self
            .definition
            .thickness
            .evaluate_as(Dimension::LENGTH, &|id| self.inputs.parameters.value(id))
            .map_err(|error| {
                let (remedy, fix) = match &error {
                    EvalError::ParameterFailed { id, name } => (
                        format!("Fix {name} under Parameters, or edit the thickness."),
                        FixTarget::Parameter(*id),
                    ),
                    EvalError::WrongKind { .. } => (
                        "Edit the thickness so it gives a length, such as 2 mm.".to_owned(),
                        FixTarget::Feature(self.feature.id()),
                    ),
                    _ => (
                        "Edit the thickness or the parameters it uses.".to_owned(),
                        FixTarget::Feature(self.feature.id()),
                    ),
                };
                Failure::Error(FeatureError {
                    reason: format!("The thickness cannot be evaluated: {error}."),
                    remedy,
                    fix: Some(fix),
                    constraints: Vec::new(),
                })
            })?;
        if value > 0.0 {
            Ok(value)
        } else {
            Err(self.error(
                "The thickness must be more than zero.".to_owned(),
                "Enter a thickness above zero.".to_owned(),
            ))
        }
    }

    fn describe_face(&self, solid: &Solid, face: FaceId) -> String {
        describe_origin(
            self.inputs.document,
            solid.face(face).and_then(|face| face.origin()),
        )
    }

    fn failure(&self, solid: &Solid, open: &[FaceId], error: &ShellError) -> Failure {
        match error {
            ShellError::InvalidThickness => self.error(
                "The thickness must be more than zero.".to_owned(),
                "Enter a thickness above zero.".to_owned(),
            ),
            ShellError::MissingFace(_) => self.error(
                format!(
                    "A face to open is no longer part of the body of {}.",
                    self.body_name
                ),
                "Choose the faces again, or undo the change that removed it.".to_owned(),
            ),
            ShellError::UnsupportedFace(face) if open.contains(face) => self.error(
                format!(
                    "Only flat faces can be opened, and {} is curved.",
                    self.describe_face(solid, *face)
                ),
                "Leave this face out.".to_owned(),
            ),
            ShellError::UnsupportedFace(face) => self.error(
                format!(
                    "The walls cannot follow {}: faces swept from splines cannot be offset yet.",
                    self.describe_face(solid, *face)
                ),
                "Shell the body before adding those faces, or open that face.".to_owned(),
            ),
            ShellError::UnsupportedEdge(edge) => self.error(
                format!(
                    "The walls cannot follow {}.",
                    describe_edge(self.inputs.document, solid, *edge)
                ),
                "Try a different thickness, or shell the body before the feature that made \
                 this edge."
                    .to_owned(),
            ),
            ShellError::TooThick => self.error(
                format!(
                    "The thickness is too large for the body of {}.",
                    self.body_name
                ),
                "Enter a smaller thickness.".to_owned(),
            ),
            ShellError::Boolean(_) => {
                log::warn!("{} could not be built: {error}", self.feature.name);
                self.error(
                    format!(
                        "The shell could not be built on the body of {}.",
                        self.body_name
                    ),
                    "Change the thickness slightly, or open different faces.".to_owned(),
                )
            }
        }
    }
}

pub(crate) fn evaluate(
    feature: &Feature,
    definition: &Shell,
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
    let thickness = context.thickness()?;
    let Some(solid) = inputs.body(definition.body) else {
        return Err(Failure::Error(FeatureError {
            reason: format!("The body made by {} has no shape.", context.body_name),
            remedy: format!("Fix {} first.", context.body_name),
            fix: Some(FixTarget::Feature(definition.body)),
            constraints: Vec::new(),
        }));
    };
    let open = definition.resolve(solid).map_err(|missing| {
        let reason = if missing == 1 {
            format!(
                "A face to open is no longer part of the body of {}.",
                context.body_name
            )
        } else {
            format!(
                "{missing} faces to open are no longer part of the body of {}.",
                context.body_name
            )
        };
        context.error(
            reason,
            "Choose the faces again, or undo the change that removed them.".to_owned(),
        )
    })?;
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let result = shell(solid, &open, thickness, feature.id().raw())
        .map_err(|error| context.failure(solid, &open, &error))?;
    Ok(FeatureResult::Solid(SolidResult::new(
        definition.body,
        result,
    )))
}
