use std::collections::BTreeSet;

use caditor_expression::{Dimension, EvalError, Expression, ParameterId};
use caditor_kernel::{
    FaceId, FaceReference, OffsetError, Solid, VertexId, offset_faces, tangent_faces,
};

use crate::{
    datum::capitalized,
    describe::{describe_edge, describe_origin, edge_faces},
    document::{Feature, FeatureId, list_names},
    origins,
    pieces::{Resolution, Unresolved, pieces_of_one_face, tally},
    recompute::{CancelToken, Failure, FeatureError, FeatureResult, FixTarget, Inputs},
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
    fn error(&self, reason: String, remedy: String) -> Failure {
        Failure::Error(Box::new(FeatureError {
            reason,
            remedy,
            fix: Some(FixTarget::Feature(self.feature.id())),
            constraints: Vec::new(),
            place: None,
        }))
    }

    fn distance(&self) -> Result<f64, Failure> {
        self.definition
            .distance
            .evaluate_as(Dimension::LENGTH, &|id| self.inputs.parameters.value(id))
            .map_err(|error| {
                let (remedy, fix) = match &error {
                    EvalError::ParameterFailed { id, name } => (
                        format!("Fix {name} under Parameters, or edit the distance."),
                        FixTarget::Parameter(*id),
                    ),
                    EvalError::WrongKind { .. } => (
                        "Edit the distance so it gives a length, such as 2 mm.".to_owned(),
                        FixTarget::Feature(self.feature.id()),
                    ),
                    _ => (
                        "Edit the distance or the parameters it uses.".to_owned(),
                        FixTarget::Feature(self.feature.id()),
                    ),
                };
                Failure::Error(Box::new(FeatureError {
                    reason: format!("The distance cannot be evaluated: {error}."),
                    remedy,
                    fix: Some(fix),
                    constraints: Vec::new(),
                    place: None,
                }))
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
        match error {
            OffsetError::Cancelled(_) => Failure::Cancelled,
            OffsetError::InvalidDistance => self.error(
                "The distance must be more than 0.000001 mm.".to_owned(),
                "Enter a larger distance, positive to grow the body and negative to shrink it."
                    .to_owned(),
            ),
            OffsetError::NothingToMove => self.error(
                "No face is chosen to move.".to_owned(),
                "Choose the faces to move.".to_owned(),
            ),
            OffsetError::MissingFace(_) => self.error(
                format!(
                    "A face to move is no longer part of the body of {}.",
                    self.body_name
                ),
                "Choose the faces again, or undo the change that removed it.".to_owned(),
            ),
            OffsetError::UnsupportedFace(face) => self.error(
                format!(
                    "{} cannot be moved yet: faces swept from splines or shaped freely cannot be \
                     offset.",
                    capitalized(&self.describe_face(solid, *face))
                ),
                "Leave this face out.".to_owned(),
            ),
            OffsetError::UnsupportedEdge(edge) => self.error(
                format!(
                    "The faces beside the moved ones cannot follow {}.",
                    describe_edge(self.inputs.document, solid, *edge)
                ),
                "Move the faces tangent to the moved ones too, or try a different distance."
                    .to_owned(),
            ),
            OffsetError::Vanishes(face) => self.error(
                format!(
                    "{} would vanish at this distance.",
                    capitalized(&self.describe_face(solid, *face))
                ),
                "Enter a smaller distance.".to_owned(),
            ),
            OffsetError::TooCurved(face) => self.error(
                format!(
                    "{} curves more tightly than the distance, so it would vanish.",
                    capitalized(&self.describe_face(solid, *face))
                ),
                "Enter a distance below the smallest radius of that face.".to_owned(),
            ),
            OffsetError::Corner(vertex) => self.error(
                format!(
                    "The faces of the body of {} cannot meet at {}.",
                    self.body_name,
                    self.describe_corner(solid, *vertex)
                ),
                "Change the distance, or move the faces at that corner too.".to_owned(),
            ),
            OffsetError::EdgeCollapses(edge) => self.error(
                format!(
                    "The face along {} would shrink to nothing at this distance.",
                    describe_edge(self.inputs.document, solid, *edge)
                ),
                "Enter a smaller distance.".to_owned(),
            ),
            OffsetError::Invalid { faces, edge } => {
                let names = self.describe_faces(solid, faces);
                let near = match (names.is_empty(), edge) {
                    (false, _) => format!(" near {}", list_names(&names)),
                    (true, Some(edge)) => format!(
                        " near {}",
                        describe_edge(self.inputs.document, solid, *edge)
                    ),
                    (true, None) => String::new(),
                };
                self.error(
                    format!(
                        "The body of {} would not stay valid{near} at this distance, as a moved \
                         face would pass through another part of it.",
                        self.body_name
                    ),
                    "Enter a smaller distance.".to_owned(),
                )
            }
            OffsetError::Crosses { first, second } => self.error(
                format!(
                    "{} and {} would cross each other at this distance.",
                    self.describe_face(solid, *first),
                    self.describe_face(solid, *second)
                ),
                "Enter a smaller distance.".to_owned(),
            ),
        }
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
        .feature(definition.body)
        .map(|body| body.name.clone())
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
        let body = &context.body_name;
        let reason = match unresolved {
            Unresolved::Missing(1) => {
                format!("A face to move is no longer part of the body of {body}.")
            }
            Unresolved::Missing(missing) => {
                format!("{missing} faces to move are no longer part of the body of {body}.")
            }
            Unresolved::Unrelated(1) => {
                format!("A face to move now matches several separate faces of the body of {body}.")
            }
            Unresolved::Unrelated(unrelated) => format!(
                "{unrelated} faces to move now match several separate faces of the body of \
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
