use std::collections::BTreeSet;

use caditor_kernel::{DeleteError, FaceId, FaceReference, Solid, delete_faces};

use crate::{
    datum::capitalized,
    describe::describe_origin,
    document::{Feature, FeatureId, list_names},
    failure::{Cause, Chosen, Unmatched},
    origins,
    pieces::{Resolution, Unresolved, pieces_of_one_face, tally},
    recompute::{
        CancelToken, Failure, FeatureError, FeatureResult, FixTarget, Inputs, MissingInput,
    },
    solid::SolidResult,
};

#[derive(Debug, Clone, PartialEq)]
pub struct DeleteFace {
    pub body: FeatureId,
    pub faces: Vec<FaceReference>,
}

impl DeleteFace {
    pub fn features(&self) -> BTreeSet<FeatureId> {
        BTreeSet::from([self.body])
    }

    pub fn origin_features(&self) -> BTreeSet<FeatureId> {
        self.faces.iter().flat_map(origins::of_face).collect()
    }

    pub fn heap_size(&self) -> usize {
        size_of_val(self.faces.as_slice())
            + self
                .faces
                .iter()
                .map(FaceReference::heap_size)
                .sum::<usize>()
    }

    pub fn resolutions(&self, solid: &Solid) -> Vec<Resolution<FaceId>> {
        self.faces
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
    definition: &'a DeleteFace,
    inputs: &'a Inputs<'a>,
    body_name: String,
}

impl Context<'_> {
    fn error(&self, cause: DeleteFaceCause) -> Failure {
        Failure::from(FeatureError::new(cause).fixing(FixTarget::Feature(self.feature.id())))
    }

    fn describe_face(&self, solid: &Solid, face: FaceId) -> String {
        describe_origin(
            self.inputs.document,
            solid.face(face).and_then(|face| face.origin()),
        )
    }

    fn describe_faces(&self, solid: &Solid, faces: &[FaceId]) -> Vec<String> {
        let mut names: Vec<String> = Vec::with_capacity(faces.len());
        for face in faces {
            let name = self.describe_face(solid, *face);
            if !names.contains(&name) {
                names.push(name);
            }
        }
        names
    }

    fn failure(&self, solid: &Solid, error: &DeleteError) -> Failure {
        let body = self.body_name.clone();
        match error {
            DeleteError::Cancelled(_) => Failure::Cancelled,
            DeleteError::NothingToDelete => Failure::Waiting(MissingInput::FacesToDelete {
                body: self.definition.body,
            }),
            DeleteError::MissingFace(_) => self.error(DeleteFaceCause::FaceGone { body }),
            DeleteError::Everything => self.error(DeleteFaceCause::Everything { body }),
            DeleteError::CannotClose(face) => self.error(DeleteFaceCause::CannotClose {
                face: self.describe_face(solid, *face),
            }),
            DeleteError::Vanishes(face) => self.error(DeleteFaceCause::Vanishes {
                face: self.describe_face(solid, *face),
            }),
            DeleteError::Invalid { faces } => self.error(DeleteFaceCause::Invalid {
                body,
                near: self.describe_faces(solid, faces),
            }),
            DeleteError::Crosses { first, second } => self.error(DeleteFaceCause::Crosses {
                first: self.describe_face(solid, *first),
                second: self.describe_face(solid, *second),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum DeleteFaceCause {
    FaceGone { body: String },
    Everything { body: String },
    CannotClose { face: String },
    Vanishes { face: String },
    Invalid { body: String, near: Vec<String> },
    Crosses { first: String, second: String },
}

impl DeleteFaceCause {
    pub(crate) fn reason(&self) -> String {
        match self {
            Self::FaceGone { body } => {
                format!("A face to delete is no longer part of the body of {body}.")
            }
            Self::Everything { body } => {
                format!("Deleting every face of the body of {body} would leave nothing.")
            }
            Self::CannotClose { face } => {
                format!("The faces around {face} cannot close the gap it leaves.")
            }
            Self::Vanishes { face } => {
                format!("{} would vanish as the gap closes.", capitalized(face))
            }
            Self::Invalid { body, near } => {
                let near = if near.is_empty() {
                    String::new()
                } else {
                    format!(" near {}", list_names(near))
                };
                format!(
                    "Closing the gap would not leave the body of {body} valid{near}, as the \
                     faces around it would have to pass through another part of it."
                )
            }
            Self::Crosses { first, second } => format!(
                "{} and {second} would cross each other once the gap closes.",
                capitalized(first)
            ),
        }
    }

    pub(crate) fn remedy(&self) -> String {
        match self {
            Self::FaceGone { .. } => {
                "Choose the faces again, or undo the change that removed it.".to_owned()
            }
            Self::Everything { .. } => {
                "Leave out the faces that should stay, or remove the body instead.".to_owned()
            }
            Self::CannotClose { .. } => {
                "Choose the faces next to it as well, such as the whole chain of a fillet, the \
                 floor of a blind hole or every face of a boss, or leave this face out."
                    .to_owned()
            }
            Self::Vanishes { .. } => {
                "Delete that face too, or leave out the faces whose gap swallows it.".to_owned()
            }
            Self::Invalid { .. } | Self::Crosses { .. } => {
                "Choose the faces next to the deleted ones as well, or leave some out.".to_owned()
            }
        }
    }
}

impl From<DeleteFaceCause> for Cause {
    fn from(cause: DeleteFaceCause) -> Self {
        Self::DeleteFace(cause)
    }
}

pub(crate) fn evaluate(
    feature: &Feature,
    definition: &DeleteFace,
    inputs: &Inputs<'_>,
    cancel: &CancelToken,
) -> Result<FeatureResult, Failure> {
    let body_name = inputs
        .document
        .body_label(definition.body)
        .unwrap_or_default();
    let context = Context {
        feature,
        definition,
        inputs,
        body_name,
    };
    let Some(solid) = inputs.body(definition.body) else {
        return Err(inputs.missing_body(definition.body));
    };
    let faces = definition.resolve(solid).map_err(|unresolved| {
        Failure::from(
            FeatureError::new(Unmatched {
                chosen: Chosen::FacesToDelete,
                unresolved,
                body: context.body_name.clone(),
            })
            .fixing(FixTarget::Feature(feature.id())),
        )
    })?;
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let result = delete_faces(solid, &faces).map_err(|error| context.failure(solid, &error))?;
    Ok(FeatureResult::Solid(SolidResult::new(
        definition.body,
        result,
    )))
}
