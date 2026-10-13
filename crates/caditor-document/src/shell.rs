use std::collections::{BTreeMap, BTreeSet};

use caditor_expression::{Dimension, EvalError, Expression, ParameterId};
use caditor_kernel::{
    FaceId, FaceReference, ShellError, Solid, Surface, VertexId, Walls, shell_walls,
};

use crate::{
    datum::capitalized,
    describe::{describe_edge, describe_origin, edge_faces},
    document::{Feature, FeatureId},
    origins,
    pieces::{Resolution, Unresolved, pieces_of_one_face, tally},
    recompute::{CancelToken, Failure, FeatureError, FeatureResult, FixTarget, Inputs},
    solid::SolidResult,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShellSide {
    #[default]
    Inside,
    Outside,
    BothSides,
}

impl ShellSide {
    pub const ALL: [Self; 3] = [Self::Inside, Self::Outside, Self::BothSides];

    pub fn label(self) -> &'static str {
        match self {
            Self::Inside => "Inside",
            Self::Outside => "Outside",
            Self::BothSides => "Both sides",
        }
    }

    pub fn explanation(self) -> &'static str {
        match self {
            Self::Inside => "The walls are hollowed out inside the faces, keeping the body's size",
            Self::Outside => {
                "The walls grow outside the faces, keeping the body's shape as the hollow"
            }
            Self::BothSides => "Half of each wall lies inside the faces and half outside",
        }
    }

    fn kernel(self) -> caditor_kernel::ShellSide {
        match self {
            Self::Inside => caditor_kernel::ShellSide::Inside,
            Self::Outside => caditor_kernel::ShellSide::Outside,
            Self::BothSides => caditor_kernel::ShellSide::BothSides,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FaceThickness {
    pub face: FaceReference,
    pub thickness: Expression,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Shell {
    pub body: FeatureId,
    pub open: Vec<FaceReference>,
    pub thickness: Expression,
    pub side: ShellSide,
    pub faces: Vec<FaceThickness>,
}

impl Shell {
    pub fn expressions(&self) -> impl Iterator<Item = &Expression> {
        std::iter::once(&self.thickness).chain(self.faces.iter().map(|face| &face.thickness))
    }

    pub fn expressions_mut(&mut self) -> Vec<&mut Expression> {
        std::iter::once(&mut self.thickness)
            .chain(self.faces.iter_mut().map(|face| &mut face.thickness))
            .collect()
    }

    pub fn heap_size(&self) -> usize {
        size_of_val(self.open.as_slice())
            + self
                .open
                .iter()
                .map(FaceReference::heap_size)
                .sum::<usize>()
            + size_of_val(self.faces.as_slice())
            + self
                .faces
                .iter()
                .map(|face| face.face.heap_size() + face.thickness.heap_size())
                .sum::<usize>()
            + self.thickness.heap_size()
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
        self.open
            .iter()
            .chain(self.faces.iter().map(|face| &face.face))
            .flat_map(origins::of_face)
            .collect()
    }

    pub fn resolutions(&self, solid: &Solid) -> Vec<Resolution<FaceId>> {
        resolutions(solid, self.open.iter())
    }

    pub fn wall_resolutions(&self, solid: &Solid) -> Vec<Resolution<FaceId>> {
        resolutions(solid, self.faces.iter().map(|face| &face.face))
    }

    pub fn resolve(&self, solid: &Solid) -> Result<Vec<FaceId>, Unresolved> {
        tally(self.resolutions(solid))
    }
}

pub fn openable_face(solid: &Solid, face: FaceId) -> bool {
    solid.face(face).is_some_and(|face| {
        matches!(
            face.surface(),
            Surface::Plane(_)
                | Surface::Cylinder(_)
                | Surface::Cone(_)
                | Surface::Sphere(_)
                | Surface::Torus(_)
        )
    })
}

fn resolutions<'a>(
    solid: &Solid,
    references: impl Iterator<Item = &'a FaceReference>,
) -> Vec<Resolution<FaceId>> {
    references
        .map(|reference| {
            Resolution::of(reference.resolve(solid), |pieces| {
                pieces_of_one_face(solid, pieces)
            })
        })
        .collect()
}

struct Context<'a> {
    feature: &'a Feature,
    definition: &'a Shell,
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

    fn length(&self, expression: &Expression, face: Option<&str>) -> Result<f64, Failure> {
        let of_face = face.map(|face| format!(" of {face}")).unwrap_or_default();
        let value = expression
            .evaluate_as(Dimension::LENGTH, &|id| self.inputs.parameters.value(id))
            .map_err(|error| {
                let (remedy, fix) = match &error {
                    EvalError::ParameterFailed { id, name } => (
                        format!("Fix {name} under Parameters, or edit the thickness{of_face}."),
                        FixTarget::Parameter(*id),
                    ),
                    EvalError::WrongKind { .. } => (
                        format!("Edit the thickness{of_face} so it gives a length, such as 2 mm."),
                        FixTarget::Feature(self.feature.id()),
                    ),
                    _ => (
                        format!("Edit the thickness{of_face} or the parameters it uses."),
                        FixTarget::Feature(self.feature.id()),
                    ),
                };
                Failure::Error(Box::new(FeatureError {
                    reason: format!("The thickness{of_face} cannot be evaluated: {error}."),
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
                format!("The thickness{of_face} must be more than zero."),
                format!("Enter a thickness{of_face} above zero."),
            ))
        }
    }

    fn walls(&self, solid: &Solid, thickness: f64) -> Result<Walls, Failure> {
        let mut faces = BTreeMap::new();
        let resolutions = self.definition.wall_resolutions(solid);
        for (entry, resolution) in self.definition.faces.iter().zip(resolutions) {
            let described = describe_origin(self.inputs.document, entry.face.origin());
            let value = self.length(&entry.thickness, Some(&described))?;
            let remedy =
                || "Choose the face again, or remove its own thickness from the shell.".to_owned();
            match resolution {
                Resolution::One(face) => {
                    faces.insert(face, value);
                }
                Resolution::Pieces(pieces) => {
                    faces.extend(pieces.into_iter().map(|face| (face, value)));
                }
                Resolution::Tied(_) => {
                    return Err(self.error(
                        format!(
                            "{} given its own thickness now matches several separate faces of \
                             the body of {}.",
                            capitalized(&described),
                            self.body_name
                        ),
                        remedy(),
                    ));
                }
                Resolution::Missing => {
                    return Err(self.error(
                        format!(
                            "{} given its own thickness is no longer part of the body of {}.",
                            capitalized(&described),
                            self.body_name
                        ),
                        remedy(),
                    ));
                }
            }
        }
        Ok(Walls {
            thickness,
            side: self.definition.side.kernel(),
            faces,
        })
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
            ShellError::InvalidFaceThickness(face) => self.error(
                format!(
                    "The thickness of {} must be more than 0.000001 mm.",
                    self.describe_face(solid, *face)
                ),
                "Enter a larger thickness for that face.".to_owned(),
            ),
            ShellError::OpenedFaceThickness(face) => self.error(
                format!(
                    "{} is open, so it has no wall to give its own thickness.",
                    capitalized(&self.describe_face(solid, *face))
                ),
                "Remove its own thickness, or close the face again.".to_owned(),
            ),
            ShellError::UnsupportedFace(face) if open.contains(face) => self.error(
                format!(
                    "{} is swept from a spline or curve, and only flat faces and cylinder, cone, \
                     sphere or torus faces can be opened.",
                    capitalized(&self.describe_face(solid, *face))
                ),
                "Leave this face out.".to_owned(),
            ),
            ShellError::CurvedOpening(face) => self.error(
                format!(
                    "{} is curved, so it can open only on the outside of the body and where \
                     sharp edges part it from the faces left closed.",
                    capitalized(&self.describe_face(solid, *face))
                ),
                "Open the faces it meets smoothly as well, or open a flat face instead.".to_owned(),
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
            ShellError::ClosesBesideClosing { face, beyond } => self.error(
                format!(
                    "{} and {} beside it both close up at this thickness, so their walls have \
                     nowhere to meet.",
                    capitalized(&self.describe_face(solid, *face)),
                    self.describe_face(solid, *beyond)
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
            ShellError::Overhang { open, wall } => self.error(
                format!(
                    "{} leans over the opening in {}, so cutting the opening at this thickness \
                     would leave its wall thinner than the rest.",
                    capitalized(&self.describe_face(solid, *wall)),
                    self.describe_face(solid, *open)
                ),
                "Enter a smaller thickness.".to_owned(),
            ),
            ShellError::Walls { faces, edge } => {
                let mut names: Vec<String> = Vec::with_capacity(faces.len());
                for face in faces {
                    let name = self.describe_face(solid, *face);
                    if !names.contains(&name) {
                        names.push(name);
                    }
                }
                let near = match (names.is_empty(), edge) {
                    (false, _) => format!(" near {}", crate::document::list_names(&names)),
                    (true, Some(edge)) => format!(
                        " near {}",
                        describe_edge(self.inputs.document, solid, *edge)
                    ),
                    (true, None) => String::new(),
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
        .body_label(definition.body)
        .unwrap_or_default();
    let context = Context {
        feature,
        definition,
        inputs,
        body_name,
    };
    let thickness = context.length(&definition.thickness, None)?;
    let Some(solid) = inputs.body(definition.body) else {
        return Err(inputs.missing_body(definition.body));
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
    let walls = context.walls(solid, thickness)?;
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let result = shell_walls(solid, &open, &walls, feature.id().raw())
        .map_err(|error| context.failure(solid, &open, &error))?;
    Ok(FeatureResult::Solid(SolidResult::new(
        definition.body,
        result,
    )))
}
