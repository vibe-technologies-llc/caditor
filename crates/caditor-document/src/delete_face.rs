use std::collections::BTreeSet;

use caditor_kernel::{DeleteError, FaceId, FaceReference, Solid, delete_faces};

use crate::{
    datum::capitalized,
    describe::describe_origin,
    document::{Feature, FeatureId, list_names},
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
    fn error(&self, reason: String, remedy: String) -> Failure {
        Failure::Error(Box::new(FeatureError {
            reason,
            remedy,
            fix: Some(FixTarget::Feature(self.feature.id())),
            constraints: Vec::new(),
            place: None,
        }))
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
        let body = &self.body_name;
        match error {
            DeleteError::Cancelled(_) => Failure::Cancelled,
            DeleteError::NothingToDelete => Failure::Waiting(MissingInput::FacesToDelete {
                body: self.definition.body,
            }),
            DeleteError::MissingFace(_) => self.error(
                format!("A face to delete is no longer part of the body of {body}."),
                "Choose the faces again, or undo the change that removed it.".to_owned(),
            ),
            DeleteError::Everything => self.error(
                format!("Deleting every face of the body of {body} would leave nothing."),
                "Leave out the faces that should stay, or remove the body instead.".to_owned(),
            ),
            DeleteError::CannotClose(face) => self.error(
                format!(
                    "The faces around {} cannot close the gap it leaves.",
                    self.describe_face(solid, *face)
                ),
                "Choose the faces next to it as well, such as the whole chain of a fillet, the \
                 floor of a blind hole or every face of a boss, or leave this face out."
                    .to_owned(),
            ),
            DeleteError::Vanishes(face) => self.error(
                format!(
                    "{} would vanish as the gap closes.",
                    capitalized(&self.describe_face(solid, *face))
                ),
                "Delete that face too, or leave out the faces whose gap swallows it.".to_owned(),
            ),
            DeleteError::Invalid { faces } => {
                let names = self.describe_faces(solid, faces);
                let near = if names.is_empty() {
                    String::new()
                } else {
                    format!(" near {}", list_names(&names))
                };
                self.error(
                    format!(
                        "Closing the gap would not leave the body of {body} valid{near}, as the \
                         faces around it would have to pass through another part of it."
                    ),
                    "Choose the faces next to the deleted ones as well, or leave some out."
                        .to_owned(),
                )
            }
            DeleteError::Crosses { first, second } => self.error(
                format!(
                    "{} and {} would cross each other once the gap closes.",
                    capitalized(&self.describe_face(solid, *first)),
                    self.describe_face(solid, *second)
                ),
                "Choose the faces next to the deleted ones as well, or leave some out.".to_owned(),
            ),
        }
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
        let body = &context.body_name;
        let reason = match unresolved {
            Unresolved::Missing(1) => {
                format!("A face to delete is no longer part of the body of {body}.")
            }
            Unresolved::Missing(missing) => {
                format!("{missing} faces to delete are no longer part of the body of {body}.")
            }
            Unresolved::Unrelated(1) => format!(
                "A face to delete now matches several separate faces of the body of {body}."
            ),
            Unresolved::Unrelated(unrelated) => format!(
                "{unrelated} faces to delete now match several separate faces of the body of \
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
    let result = delete_faces(solid, &faces).map_err(|error| context.failure(solid, &error))?;
    Ok(FeatureResult::Solid(SolidResult::new(
        definition.body,
        result,
    )))
}
