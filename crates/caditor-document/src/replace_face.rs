use std::collections::BTreeSet;

use caditor_kernel::{
    EdgeId, FaceId, FaceReference, MoveError, ReplaceError, Solid, Surface, replace_faces,
};

use crate::{
    datum::{PlaneReference, Resolver, capitalized, describe_plane},
    describe::{describe_corner, describe_edge, describe_face, describe_faces},
    document::{Feature, FeatureId},
    failure::{Cause, Chosen, Near, Unmatched},
    origins,
    pieces::{Resolution, Unresolved, pieces_of_one_face, tally},
    recompute::{
        CancelToken, Failure, FeatureError, FeatureResult, FixTarget, Inputs, MissingInput,
    },
    solid::SolidResult,
};

#[derive(Debug, Clone, PartialEq)]
pub struct ReplaceFace {
    pub body: FeatureId,
    pub faces: Vec<FaceReference>,
    pub target: Option<PlaneReference>,
}

impl ReplaceFace {
    pub fn heap_size(&self) -> usize {
        size_of_val(self.faces.as_slice())
            + self
                .faces
                .iter()
                .map(FaceReference::heap_size)
                .sum::<usize>()
            + self.target.as_ref().map_or(0, PlaneReference::heap_size)
    }

    pub fn features(&self) -> BTreeSet<FeatureId> {
        let mut used = BTreeSet::from([self.body]);
        if let Some(target) = &self.target {
            used.extend(target.datum());
            used.extend(target.frame());
            used.extend(target.body());
        }
        used
    }

    pub fn origin_features(&self) -> BTreeSet<FeatureId> {
        let mut origins = self
            .target
            .as_ref()
            .map(PlaneReference::origin_features)
            .unwrap_or_default();
        origins.extend(self.faces.iter().flat_map(origins::of_face));
        origins
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

pub fn is_flat(solid: &Solid, face: FaceId) -> bool {
    solid
        .face(face)
        .is_some_and(|definition| matches!(definition.surface(), Surface::Plane(_)))
}

struct Context<'a> {
    feature: &'a Feature,
    definition: &'a ReplaceFace,
    inputs: &'a Inputs<'a>,
    body_name: String,
}

impl Context<'_> {
    fn error(&self, cause: ReplaceFaceCause) -> Failure {
        Failure::from(FeatureError::new(cause).fixing(FixTarget::Feature(self.feature.id())))
    }

    fn describe(&self, solid: &Solid, face: FaceId) -> String {
        describe_face(self.inputs.document, solid, face)
    }

    fn target(&self) -> String {
        self.definition.target.as_ref().map_or_else(
            || "the target".to_owned(),
            |target| describe_plane(self.inputs.document, target),
        )
    }

    fn failure(&self, solid: &Solid, error: &ReplaceError) -> Failure {
        let body = self.body_name.clone();
        let face = |face: &FaceId| self.describe(solid, *face);
        match error {
            ReplaceError::NothingToReplace => Failure::Waiting(MissingInput::FacesToReplace {
                body: self.definition.body,
            }),
            ReplaceError::MissingFace(_) => self.error(ReplaceFaceCause::FaceGone { body }),
            ReplaceError::NotFlat(of) => self.error(ReplaceFaceCause::NotFlat { face: face(of) }),
            ReplaceError::SquareToTarget(of) => self.error(ReplaceFaceCause::SquareToTarget {
                face: face(of),
                target: self.target(),
            }),
            ReplaceError::BeyondBody(of) => self.error(ReplaceFaceCause::BeyondBody {
                face: face(of),
                target: self.target(),
            }),
            ReplaceError::Merging { first, second } => self.error(ReplaceFaceCause::Merging {
                first: face(first),
                second: face(second),
            }),
            ReplaceError::AlreadyOnTarget => self.error(ReplaceFaceCause::AlreadyOnTarget {
                target: self.target(),
            }),
            ReplaceError::Move(error) => self.move_failure(solid, error),
        }
    }

    fn move_failure(&self, solid: &Solid, error: &MoveError) -> Failure {
        let document = self.inputs.document;
        let body = self.body_name.clone();
        let face = |face: &FaceId| self.describe(solid, *face);
        let edge = |edge: &EdgeId| describe_edge(document, solid, *edge);
        match error {
            MoveError::Cancelled(_) => Failure::Cancelled,
            MoveError::Unmoved => self.error(ReplaceFaceCause::AlreadyOnTarget {
                target: self.target(),
            }),
            MoveError::NothingToMove => Failure::Waiting(MissingInput::FacesToReplace {
                body: self.definition.body,
            }),
            MoveError::MissingFace(_) => self.error(ReplaceFaceCause::FaceGone { body }),
            MoveError::UnsupportedFace(of) => {
                self.error(ReplaceFaceCause::NotFlat { face: face(of) })
            }
            MoveError::UnsupportedEdge(of) => {
                self.error(ReplaceFaceCause::UnsupportedEdge { edge: edge(of) })
            }
            MoveError::Vanishes(of) => self.error(ReplaceFaceCause::Vanishes { face: face(of) }),
            MoveError::Corner(vertex) => self.error(ReplaceFaceCause::Corner {
                body,
                corner: describe_corner(document, solid, *vertex),
            }),
            MoveError::EdgeCollapses(of) => {
                self.error(ReplaceFaceCause::EdgeCollapses { edge: edge(of) })
            }
            MoveError::Invalid { faces, edge: near } => self.error(ReplaceFaceCause::Invalid {
                body,
                near: Near {
                    faces: describe_faces(document, solid, faces),
                    edge: near.as_ref().map(edge),
                },
            }),
            MoveError::Crosses { first, second } => self.error(ReplaceFaceCause::Crosses {
                first: face(first),
                second: face(second),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ReplaceFaceCause {
    FaceGone { body: String },
    NotFlat { face: String },
    SquareToTarget { face: String, target: String },
    BeyondBody { face: String, target: String },
    Merging { first: String, second: String },
    AlreadyOnTarget { target: String },
    UnsupportedEdge { edge: String },
    Vanishes { face: String },
    Corner { body: String, corner: String },
    EdgeCollapses { edge: String },
    Invalid { body: String, near: Near },
    Crosses { first: String, second: String },
}

impl ReplaceFaceCause {
    pub(crate) fn reason(&self) -> String {
        match self {
            Self::FaceGone { body } => {
                format!("A face to replace is no longer part of the body of {body}.")
            }
            Self::NotFlat { face } => format!(
                "{} is curved; only flat faces can be replaced.",
                capitalized(face)
            ),
            Self::SquareToTarget { face, target } => format!(
                "{} runs square to {}, so it cannot be turned onto it.",
                capitalized(face),
                target
            ),
            Self::BeyondBody { face, target } => format!(
                "{} lies on or beyond the far side of the body from {}, so replacing it would \
                 turn the body inside out.",
                capitalized(target),
                face
            ),
            Self::Merging { first, second } => format!(
                "{} meets {} along an edge, and both would lie on the target, joining them into \
                 one face.",
                capitalized(second),
                first
            ),
            Self::AlreadyOnTarget { target } => {
                format!("Every chosen face already lies on {target}.")
            }
            Self::UnsupportedEdge { edge } => {
                format!("The faces beside the replaced ones cannot follow {edge}.")
            }
            Self::Vanishes { face } => {
                format!("{} would vanish with this replacement.", capitalized(face))
            }
            Self::Corner { body, corner } => {
                format!("The faces of the body of {body} cannot meet at {corner}.")
            }
            Self::EdgeCollapses { edge } => {
                format!("The face along {edge} would shrink to nothing with this replacement.")
            }
            Self::Invalid { body, near } => format!(
                "The body of {body} would not stay valid{near} with this replacement, as a \
                 replaced face would pass through another part of it."
            ),
            Self::Crosses { first, second } => {
                format!("{first} and {second} would cross each other with this replacement.")
            }
        }
    }

    pub(crate) fn remedy(&self) -> String {
        match self {
            Self::FaceGone { .. } => {
                "Choose the faces again, or undo the change that removed it.".to_owned()
            }
            Self::NotFlat { .. } => "Leave this face out.".to_owned(),
            Self::SquareToTarget { .. } => {
                "Leave this face out, or choose a target that leans less from it.".to_owned()
            }
            Self::BeyondBody { .. } => {
                "Choose a target that crosses the body or lies outside this face.".to_owned()
            }
            Self::Merging { .. } => {
                "Delete the face between them first, or replace only one of them.".to_owned()
            }
            Self::AlreadyOnTarget { .. } => "Choose another target, or other faces.".to_owned(),
            Self::UnsupportedEdge { .. } | Self::Corner { .. } => {
                "Replace the faces before rounding the edges beside them, or replace the faces \
                 beside them too."
                    .to_owned()
            }
            Self::Vanishes { .. }
            | Self::EdgeCollapses { .. }
            | Self::Invalid { .. }
            | Self::Crosses { .. } => {
                "Choose a target closer to the faces, or delete the face that would vanish first."
                    .to_owned()
            }
        }
    }
}

impl From<ReplaceFaceCause> for Cause {
    fn from(cause: ReplaceFaceCause) -> Self {
        Self::ReplaceFace(cause)
    }
}

pub(crate) fn evaluate(
    feature: &Feature,
    definition: &ReplaceFace,
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
                chosen: Chosen::FacesToReplace,
                unresolved,
                body: context.body_name.clone(),
            })
            .fixing(FixTarget::Feature(feature.id())),
        )
    })?;
    if faces.is_empty() {
        return Err(Failure::Waiting(MissingInput::FacesToReplace {
            body: definition.body,
        }));
    }
    let Some(target) = &definition.target else {
        return Err(Failure::Waiting(MissingInput::TargetPlane {
            body: definition.body,
        }));
    };
    let target = Resolver { feature, inputs }.plane(target)?;
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let result =
        replace_faces(solid, &faces, &target).map_err(|error| context.failure(solid, &error))?;
    Ok(FeatureResult::Solid(SolidResult::new(
        definition.body,
        result,
    )))
}
