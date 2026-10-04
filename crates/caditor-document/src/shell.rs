use std::collections::BTreeSet;

use caditor_expression::{Dimension, EvalError, Expression, ParameterId};
use caditor_kernel::{FaceId, FaceReference, ShellError, Solid, VertexId, shell};

use crate::{
    datum::capitalized,
    describe::{describe_edge, describe_origin, edge_faces},
    document::{Feature, FeatureId},
    origins,
    pieces::{Resolution, Unresolved, pieces_of_one_face, tally},
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

    pub fn origin_features(&self) -> BTreeSet<FeatureId> {
        self.open.iter().flat_map(origins::of_face).collect()
    }

    pub fn resolutions(&self, solid: &Solid) -> Vec<Resolution<FaceId>> {
        self.open
            .iter()
            .map(|reference| {
                Resolution::of(reference.resolve(solid), |pieces| {
                    pieces_of_one_face(solid, pieces)
                })
            })
            .collect()
    }

    pub fn resolve(&self, solid: &Solid) -> Result<Vec<FaceId>, Unresolved> {
        tally(self.resolutions(solid))
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

    fn describe_corner(&self, solid: &Solid, vertex: VertexId) -> String {
        let faces: BTreeSet<FaceId> = solid
            .edges()
            .filter(|(_, edge)| edge.start() == vertex || edge.end() == vertex)
            .flat_map(|(id, _)| edge_faces(solid, id))
            .collect();
        let mut names: Vec<String> = Vec::with_capacity(faces.len());
        for face in faces {
            let name = self.describe_face(solid, face);
            if !names.contains(&name) {
                names.push(name);
            }
        }
        match names.as_slice() {
            [] => "one of its corners".to_owned(),
            [only] => format!("the corner of {only}"),
            [rest @ .., last] => format!("the corner where {} and {last} meet", rest.join(", ")),
        }
    }

    fn failure(&self, solid: &Solid, open: &[FaceId], error: &ShellError) -> Failure {
        match error {
            ShellError::Cancelled(_) => Failure::Cancelled,
            ShellError::InvalidThickness => self.error(
                "The thickness must be more than 0.000001 mm.".to_owned(),
                "Enter a larger thickness.".to_owned(),
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
            ShellError::TooCurved(face) => self.error(
                format!(
                    "{} curves more tightly than the thickness, so its wall would vanish.",
                    capitalized(&self.describe_face(solid, *face))
                ),
                "Enter a thickness below the smallest radius of that face.".to_owned(),
            ),
            ShellError::Corner(vertex) => self.error(
                format!(
                    "The walls of the body of {} cannot meet at {}.",
                    self.body_name,
                    self.describe_corner(solid, *vertex)
                ),
                "Change the thickness, or open one of the faces at that corner.".to_owned(),
            ),
            ShellError::EdgeCollapses(edge) => self.error(
                format!(
                    "The wall along {} shrinks to nothing at this thickness.",
                    describe_edge(self.inputs.document, solid, *edge)
                ),
                "Enter a smaller thickness.".to_owned(),
            ),
            ShellError::Opening(face) => self.error(
                format!(
                    "The opening in {} could not be cut at this thickness.",
                    self.describe_face(solid, *face)
                ),
                "Enter a smaller thickness, or leave this face closed.".to_owned(),
            ),
            ShellError::Walls { face, edge } => {
                let near = match (face, edge) {
                    (Some(face), _) => format!(" near {}", self.describe_face(solid, *face)),
                    (None, Some(edge)) => format!(
                        " near {}",
                        describe_edge(self.inputs.document, solid, *edge)
                    ),
                    (None, None) => String::new(),
                };
                self.error(
                    format!(
                        "The walls of the body of {} would cross each other{near} at this \
                         thickness.",
                        self.body_name
                    ),
                    "Enter a smaller thickness.".to_owned(),
                )
            }
            ShellError::TooThick => self.error(
                format!(
                    "The thickness is too large for the body of {}.",
                    self.body_name
                ),
                "Enter a smaller thickness.".to_owned(),
            ),
            ShellError::Voids(_) => {
                log::warn!("{} could not be built: {error}", self.feature.name);
                self.error(
                    format!(
                        "The body of {} could not be divided into its outside and its hollows, \
                         so it cannot be shelled.",
                        self.body_name
                    ),
                    "Change the feature that made the body slightly, or shell it before that \
                     feature."
                        .to_owned(),
                )
            }
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
    let open = definition.resolve(solid).map_err(|unresolved| {
        let body = &context.body_name;
        let reason = match unresolved {
            Unresolved::Missing(1) => {
                format!("A face to open is no longer part of the body of {body}.")
            }
            Unresolved::Missing(missing) => {
                format!("{missing} faces to open are no longer part of the body of {body}.")
            }
            Unresolved::Unrelated(1) => {
                format!("A face to open now matches several separate faces of the body of {body}.")
            }
            Unresolved::Unrelated(unrelated) => format!(
                "{unrelated} faces to open now match several separate faces of the body of \
                 {body}."
            ),
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
