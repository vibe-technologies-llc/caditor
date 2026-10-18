use std::collections::BTreeSet;

use caditor_expression::{Dimension, Expression, ParameterId};
use caditor_geometry::{RigidTransform, Vector3};
use caditor_kernel::{
    EdgeId, FaceId, FaceReference, MoveError, Solid, Surface, VertexId, move_faces,
};

use crate::{
    datum::{AxisReference, Resolver, capitalized},
    describe::{describe_corner, describe_edge, describe_face, describe_faces},
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
pub struct MoveFace {
    pub body: FeatureId,
    pub faces: Vec<FaceReference>,
    pub distance: Expression,
    pub direction: Option<AxisReference>,
    pub angle: Expression,
    pub axis: Option<AxisReference>,
}

impl MoveFace {
    pub fn expressions(&self) -> [&Expression; 2] {
        [&self.distance, &self.angle]
    }

    pub fn expressions_mut(&mut self) -> [&mut Expression; 2] {
        [&mut self.distance, &mut self.angle]
    }

    pub fn references(&self) -> impl Iterator<Item = &AxisReference> {
        self.direction.iter().chain(self.axis.iter())
    }

    pub fn references_mut(&mut self) -> impl Iterator<Item = &mut AxisReference> {
        self.direction.iter_mut().chain(self.axis.iter_mut())
    }

    pub fn heap_size(&self) -> usize {
        size_of_val(self.faces.as_slice())
            + self
                .faces
                .iter()
                .map(FaceReference::heap_size)
                .sum::<usize>()
            + self
                .expressions()
                .into_iter()
                .map(Expression::heap_size)
                .sum::<usize>()
            + self
                .references()
                .map(AxisReference::heap_size)
                .sum::<usize>()
    }

    pub fn parameters(&self) -> BTreeSet<ParameterId> {
        self.expressions()
            .into_iter()
            .flat_map(Expression::parameters)
            .collect()
    }

    pub fn uses_parameter(&self, parameter: ParameterId) -> bool {
        self.expressions()
            .into_iter()
            .any(|expression| expression.uses(parameter))
    }

    pub fn reference_bodies(&self) -> BTreeSet<FeatureId> {
        self.references().filter_map(AxisReference::body).collect()
    }

    pub fn axis_datums(&self) -> BTreeSet<FeatureId> {
        self.references().filter_map(AxisReference::datum).collect()
    }

    pub fn frames(&self) -> BTreeSet<FeatureId> {
        self.references().filter_map(AxisReference::frame).collect()
    }

    pub fn sketches(&self) -> BTreeSet<FeatureId> {
        self.references()
            .filter_map(AxisReference::sketch)
            .collect()
    }

    pub fn sketch_entities(&self) -> impl Iterator<Item = (FeatureId, caditor_sketch::EntityId)> {
        self.references().filter_map(AxisReference::sketch_entity)
    }

    pub fn features(&self) -> BTreeSet<FeatureId> {
        let mut used = BTreeSet::from([self.body]);
        used.extend(self.reference_bodies());
        used.extend(self.axis_datums());
        used.extend(self.sketches());
        used
    }

    pub fn origin_features(&self) -> BTreeSet<FeatureId> {
        self.faces
            .iter()
            .flat_map(origins::of_face)
            .chain(self.references().flat_map(AxisReference::origin_features))
            .collect()
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

pub fn flat_normal(solid: &Solid, face: FaceId) -> Option<Vector3> {
    let definition = solid.face(face)?;
    match definition.surface() {
        Surface::Plane(plane) => Some(plane.frame().normal() * definition.sense().sign()),
        _ => None,
    }
}

struct Context<'a> {
    feature: &'a Feature,
    definition: &'a MoveFace,
    inputs: &'a Inputs<'a>,
    body_name: String,
}

impl Context<'_> {
    fn error(&self, cause: MoveFaceCause) -> Failure {
        Failure::from(FeatureError::new(cause).fixing(FixTarget::Feature(self.feature.id())))
    }

    fn value(&self, expression: &Expression, dimension: Dimension) -> Result<f64, Failure> {
        let (what, expected) = if dimension == Dimension::ANGLE {
            ("angle", Expected::Angle(5.0))
        } else {
            ("distance", Expected::Length(2.0))
        };
        expression
            .evaluate_as(dimension, &|id| self.inputs.parameters.value(id))
            .map_err(|error| {
                Failure::from(FeatureError::unevaluable(
                    self.feature.id(),
                    what,
                    expected,
                    error,
                ))
            })
    }

    fn resolver(&self) -> Resolver<'_> {
        Resolver {
            feature: self.feature,
            inputs: self.inputs,
        }
    }

    fn describe_face(&self, solid: &Solid, face: FaceId) -> String {
        describe_face(self.inputs.document, solid, face)
    }

    fn describe_faces(&self, solid: &Solid, faces: &[FaceId]) -> Vec<String> {
        describe_faces(self.inputs.document, solid, faces)
    }

    fn describe_corner(&self, solid: &Solid, vertex: VertexId) -> String {
        describe_corner(self.inputs.document, solid, vertex)
    }

    fn shift(&self, solid: &Solid, faces: &[FaceId], distance: f64) -> Result<Vector3, Failure> {
        if distance == 0.0 {
            return Ok(Vector3::ZERO);
        }
        let along = match &self.definition.direction {
            Some(reference) => self.resolver().axis(reference)?.direction(),
            None => {
                let chosen: Vec<FaceId> = self
                    .definition
                    .resolutions(solid)
                    .iter()
                    .flat_map(|resolution| resolution.found().iter().copied())
                    .collect();
                chosen
                    .iter()
                    .find_map(|face| flat_normal(solid, *face))
                    .ok_or_else(|| {
                        self.error(MoveFaceCause::NoNormal {
                            face: chosen
                                .first()
                                .or(faces.first())
                                .map(|face| self.describe_face(solid, *face))
                                .unwrap_or_default(),
                        })
                    })?
            }
        };
        let along = along
            .try_normalize()
            .ok_or_else(|| self.error(MoveFaceCause::Unmoved))?;
        Ok(along * distance)
    }

    fn turn(&self, angle: f64) -> Result<Option<RigidTransform>, Failure> {
        if angle == 0.0 {
            return Ok(None);
        }
        let reference = self
            .definition
            .axis
            .as_ref()
            .ok_or_else(|| self.error(MoveFaceCause::NoAxis))?;
        let axis = self.resolver().axis(reference)?;
        RigidTransform::rotation_about(axis.origin(), axis.direction(), angle.to_radians())
            .map(Some)
            .ok_or_else(|| self.error(MoveFaceCause::Unmoved))
    }

    fn motion(&self, solid: &Solid, faces: &[FaceId]) -> Result<RigidTransform, Failure> {
        let distance = self.value(&self.definition.distance, Dimension::LENGTH)?;
        let angle = self.value(&self.definition.angle, Dimension::ANGLE)?;
        let shift = self.shift(solid, faces, distance)?;
        let shifted =
            RigidTransform::translation(shift).ok_or_else(|| self.error(MoveFaceCause::Unmoved))?;
        Ok(match self.turn(angle)? {
            Some(turn) => turn.then(&shifted),
            None => shifted,
        })
    }

    fn failure(&self, solid: &Solid, error: &MoveError) -> Failure {
        let body = self.body_name.clone();
        let face = |face: &FaceId| self.describe_face(solid, *face);
        let edge = |edge: &EdgeId| describe_edge(self.inputs.document, solid, *edge);
        match error {
            MoveError::Cancelled(_) => Failure::Cancelled,
            MoveError::Unmoved => self.error(MoveFaceCause::Unmoved),
            MoveError::NothingToMove => Failure::Waiting(MissingInput::FacesToMove {
                body: self.definition.body,
            }),
            MoveError::MissingFace(_) => self.error(MoveFaceCause::FaceGone { body }),
            MoveError::UnsupportedFace(of) => {
                self.error(MoveFaceCause::UnsupportedFace { face: face(of) })
            }
            MoveError::UnsupportedEdge(of) => {
                self.error(MoveFaceCause::UnsupportedEdge { edge: edge(of) })
            }
            MoveError::Vanishes(of) => self.error(MoveFaceCause::Vanishes { face: face(of) }),
            MoveError::Corner(vertex) => self.error(MoveFaceCause::Corner {
                body,
                corner: self.describe_corner(solid, *vertex),
            }),
            MoveError::EdgeCollapses(of) => {
                self.error(MoveFaceCause::EdgeCollapses { edge: edge(of) })
            }
            MoveError::Invalid { faces, edge: near } => self.error(MoveFaceCause::Invalid {
                body,
                near: Near {
                    faces: self.describe_faces(solid, faces),
                    edge: near.as_ref().map(edge),
                },
            }),
            MoveError::Crosses { first, second } => self.error(MoveFaceCause::Crosses {
                first: face(first),
                second: face(second),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum MoveFaceCause {
    Unmoved,
    NoNormal { face: String },
    NoAxis,
    FaceGone { body: String },
    UnsupportedFace { face: String },
    UnsupportedEdge { edge: String },
    Vanishes { face: String },
    Corner { body: String, corner: String },
    EdgeCollapses { edge: String },
    Invalid { body: String, near: Near },
    Crosses { first: String, second: String },
}

impl MoveFaceCause {
    pub(crate) fn reason(&self) -> String {
        match self {
            Self::Unmoved => "The faces would not move: the distance and the angle are both \
                              nought."
                .to_owned(),
            Self::NoNormal { face } => format!(
                "{} is curved and no flat face is chosen, so there is no single direction to move \
                 along.",
                capitalized(face)
            ),
            Self::NoAxis => {
                "The faces are to tilt, but no axis to tilt them about is chosen.".to_owned()
            }
            Self::FaceGone { body } => {
                format!("A face to move is no longer part of the body of {body}.")
            }
            Self::UnsupportedFace { face } => {
                format!("{} cannot be moved.", capitalized(face))
            }
            Self::UnsupportedEdge { edge } => {
                format!("The faces beside the moved ones cannot follow {edge}.")
            }
            Self::Vanishes { face } => {
                format!("{} would vanish with this move.", capitalized(face))
            }
            Self::Corner { body, corner } => {
                format!("The faces of the body of {body} cannot meet at {corner}.")
            }
            Self::EdgeCollapses { edge } => {
                format!("The face along {edge} would shrink to nothing with this move.")
            }
            Self::Invalid { body, near } => format!(
                "The body of {body} would not stay valid{near} with this move, as a moved face \
                 would pass through another part of it."
            ),
            Self::Crosses { first, second } => {
                format!("{first} and {second} would cross each other with this move.")
            }
        }
    }

    pub(crate) fn remedy(&self) -> String {
        match self {
            Self::Unmoved => {
                "Enter a distance, or an angle and an axis to tilt the faces about.".to_owned()
            }
            Self::NoNormal { .. } => {
                "Choose a direction: an axis, a straight edge or a round face's axis.".to_owned()
            }
            Self::NoAxis => {
                "Choose an axis or a straight edge to tilt about, or set the angle to 0 deg."
                    .to_owned()
            }
            Self::FaceGone { .. } => {
                "Choose the faces again, or undo the change that removed it.".to_owned()
            }
            Self::UnsupportedFace { .. } => "Leave this face out.".to_owned(),
            Self::UnsupportedEdge { .. } | Self::Corner { .. } => {
                "Move the faces beside it too, such as a fillet or chamfer along it, or move less."
                    .to_owned()
            }
            Self::Vanishes { .. }
            | Self::EdgeCollapses { .. }
            | Self::Invalid { .. }
            | Self::Crosses { .. } => "Enter a smaller distance or angle.".to_owned(),
        }
    }
}

impl From<MoveFaceCause> for Cause {
    fn from(cause: MoveFaceCause) -> Self {
        Self::MoveFace(cause)
    }
}

pub(crate) fn evaluate(
    feature: &Feature,
    definition: &MoveFace,
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
                chosen: Chosen::FacesToMove,
                unresolved,
                body: context.body_name.clone(),
            })
            .fixing(FixTarget::Feature(feature.id())),
        )
    })?;
    if faces.is_empty() {
        return Err(Failure::Waiting(MissingInput::FacesToMove {
            body: definition.body,
        }));
    }
    let motion = context.motion(solid, &faces)?;
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let result =
        move_faces(solid, &faces, &motion).map_err(|error| context.failure(solid, &error))?;
    Ok(FeatureResult::Solid(SolidResult::new(
        definition.body,
        result,
    )))
}
