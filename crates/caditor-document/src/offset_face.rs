use std::collections::BTreeSet;

use caditor_expression::{Dimension, Expression, ParameterId};
use caditor_kernel::{
    EdgeId, FaceId, FaceReference, OffsetError, Solid, VertexId, offset_faces, tangent_faces,
};

use crate::{
    datum::capitalized,
    describe::{describe_edge, describe_origin, edge_faces},
    document::{Feature, FeatureId},
    failure::{Cause, Chosen, Expected, Near, Unmatched},
    origins,
    pieces::{Resolution, Unresolved, pieces_of_one_face, tally},
    recompute::{
        CancelToken, Failure, FeatureError, FeatureResult, FixTarget, Inputs, MissingInput,
    },
    solid::SolidResult,
};

#[derive(Debug, Clone, PartialEq)]
pub struct OffsetFace {
    pub body: FeatureId,
    pub faces: Vec<FaceReference>,
    pub distance: Expression,
    pub tangent: bool,
}

impl OffsetFace {
    pub fn parameters(&self) -> BTreeSet<ParameterId> {
        self.distance.parameters().into_iter().collect()
    }

    pub fn uses_parameter(&self, parameter: ParameterId) -> bool {
        self.distance.uses(parameter)
    }

    pub fn features(&self) -> BTreeSet<FeatureId> {
        BTreeSet::from([self.body])
    }

    pub fn origin_features(&self) -> BTreeSet<FeatureId> {
        self.faces.iter().flat_map(origins::of_face).collect()
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
    definition: &'a OffsetFace,
    inputs: &'a Inputs<'a>,
    body_name: String,
}

impl Context<'_> {
    fn error(&self, cause: OffsetFaceCause) -> Failure {
        Failure::from(FeatureError::new(cause).fixing(FixTarget::Feature(self.feature.id())))
    }

    fn distance(&self) -> Result<f64, Failure> {
        self.definition
            .distance
            .evaluate_as(Dimension::LENGTH, &|id| self.inputs.parameters.value(id))
            .map_err(|error| {
                Failure::from(FeatureError::unevaluable(
                    self.feature.id(),
                    "distance",
                    Expected::Length(2.0),
                    error,
                ))
            })
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

    fn describe_corner(&self, solid: &Solid, vertex: VertexId) -> String {
        let faces: BTreeSet<FaceId> = solid
            .edges()
            .filter(|(_, edge)| edge.start() == vertex || edge.end() == vertex)
            .flat_map(|(id, _)| edge_faces(solid, id))
            .collect();
        let faces: Vec<FaceId> = faces.into_iter().collect();
        match self.describe_faces(solid, &faces).as_slice() {
            [] => "one of its corners".to_owned(),
            [only] => format!("the corner of {only}"),
            [rest @ .., last] => format!("the corner where {} and {last} meet", rest.join(", ")),
        }
    }

    fn failure(&self, solid: &Solid, error: &OffsetError) -> Failure {
        let body = self.body_name.clone();
        let face = |face: &FaceId| self.describe_face(solid, *face);
        let edge = |edge: &EdgeId| describe_edge(self.inputs.document, solid, *edge);
        match error {
            OffsetError::Cancelled(_) => Failure::Cancelled,
            OffsetError::InvalidDistance => self.error(OffsetFaceCause::DistanceTooSmall),
            OffsetError::NothingToMove => Failure::Waiting(MissingInput::FacesToMove {
                body: self.definition.body,
            }),
            OffsetError::MissingFace(_) => self.error(OffsetFaceCause::FaceGone { body }),
            OffsetError::UnsupportedFace(of) => {
                self.error(OffsetFaceCause::UnsupportedFace { face: face(of) })
            }
            OffsetError::UnsupportedEdge(of) => {
                self.error(OffsetFaceCause::UnsupportedEdge { edge: edge(of) })
            }
            OffsetError::Vanishes(of) => self.error(OffsetFaceCause::Vanishes { face: face(of) }),
            OffsetError::TooCurved(of) => self.error(OffsetFaceCause::TooCurved { face: face(of) }),
            OffsetError::Corner(vertex) => self.error(OffsetFaceCause::Corner {
                body,
                corner: self.describe_corner(solid, *vertex),
            }),
            OffsetError::EdgeCollapses(of) => {
                self.error(OffsetFaceCause::EdgeCollapses { edge: edge(of) })
            }
            OffsetError::Invalid { faces, edge: near } => self.error(OffsetFaceCause::Invalid {
                body,
                near: Near {
                    faces: self.describe_faces(solid, faces),
                    edge: near.as_ref().map(edge),
                },
            }),
            OffsetError::Crosses { first, second } => self.error(OffsetFaceCause::Crosses {
                first: face(first),
                second: face(second),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum OffsetFaceCause {
    DistanceTooSmall,
    FaceGone { body: String },
    UnsupportedFace { face: String },
    UnsupportedEdge { edge: String },
    Vanishes { face: String },
    TooCurved { face: String },
    Corner { body: String, corner: String },
    EdgeCollapses { edge: String },
    Invalid { body: String, near: Near },
    Crosses { first: String, second: String },
}

impl OffsetFaceCause {
    pub(crate) fn reason(&self) -> String {
        match self {
            Self::DistanceTooSmall => "The distance must be more than 0.000001 mm.".to_owned(),
            Self::FaceGone { body } => {
                format!("A face to move is no longer part of the body of {body}.")
            }
            Self::UnsupportedFace { face } => format!(
                "{} cannot be moved yet: faces swept from splines or shaped freely cannot be \
                 offset.",
                capitalized(face)
            ),
            Self::UnsupportedEdge { edge } => {
                format!("The faces beside the moved ones cannot follow {edge}.")
            }
            Self::Vanishes { face } => {
                format!("{} would vanish at this distance.", capitalized(face))
            }
            Self::TooCurved { face } => format!(
                "{} curves more tightly than the distance, so it would vanish.",
                capitalized(face)
            ),
            Self::Corner { body, corner } => {
                format!("The faces of the body of {body} cannot meet at {corner}.")
            }
            Self::EdgeCollapses { edge } => {
                format!("The face along {edge} would shrink to nothing at this distance.")
            }
            Self::Invalid { body, near } => format!(
                "The body of {body} would not stay valid{near} at this distance, as a moved face \
                 would pass through another part of it."
            ),
            Self::Crosses { first, second } => {
                format!("{first} and {second} would cross each other at this distance.")
            }
        }
    }

    pub(crate) fn remedy(&self) -> String {
        match self {
            Self::DistanceTooSmall => {
                "Enter a larger distance, positive to grow the body and negative to shrink it."
                    .to_owned()
            }
            Self::FaceGone { .. } => {
                "Choose the faces again, or undo the change that removed it.".to_owned()
            }
            Self::UnsupportedFace { .. } => "Leave this face out.".to_owned(),
            Self::UnsupportedEdge { .. } => {
                "Move the faces tangent to the moved ones too, or try a different distance."
                    .to_owned()
            }
            Self::TooCurved { .. } => {
                "Enter a distance below the smallest radius of that face.".to_owned()
            }
            Self::Corner { .. } => {
                "Change the distance, or move the faces at that corner too.".to_owned()
            }
            Self::Vanishes { .. }
            | Self::EdgeCollapses { .. }
            | Self::Invalid { .. }
            | Self::Crosses { .. } => "Enter a smaller distance.".to_owned(),
        }
    }
}

impl From<OffsetFaceCause> for Cause {
    fn from(cause: OffsetFaceCause) -> Self {
        Self::OffsetFace(cause)
    }
}

pub(crate) fn evaluate(
    feature: &Feature,
    definition: &OffsetFace,
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
    let distance = context.distance()?;
    let Some(solid) = inputs.body(definition.body) else {
        return Err(inputs.missing_body(definition.body));
    };
    let faces = definition.resolve(solid).map_err(|unresolved| {
        Failure::from(
            FeatureError::new(Unmatched {
                chosen: Chosen::FacesToMove,
                unresolved,
                body: context.body_name.clone(),
            })
            .fixing(FixTarget::Feature(feature.id())),
        )
    })?;
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let faces = if definition.tangent {
        tangent_faces(solid, &faces)
    } else {
        faces
    };
    let result =
        offset_faces(solid, &faces, distance).map_err(|error| context.failure(solid, &error))?;
    Ok(FeatureResult::Solid(SolidResult::new(
        definition.body,
        result,
    )))
}
