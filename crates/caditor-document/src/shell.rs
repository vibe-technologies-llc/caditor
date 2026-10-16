use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use caditor_expression::{Dimension, Expression, ParameterId};
use caditor_kernel::{
    EdgeId, FaceId, FaceName, FaceOrigin, FaceReference, ShellError, Solid, Surface, VertexId,
    Walls, shell_walls,
};

use crate::{
    datum::capitalized,
    describe::{describe_edge, describe_origin, edge_faces},
    document::{Feature, FeatureId},
    failure::{Cause, Chosen, Expected, Near, Unmatched},
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NamedFace {
    pub name: FaceName,
    pub origin: Option<FaceOrigin>,
}

impl NamedFace {
    fn of(solid: &Solid, face: FaceId) -> Option<Self> {
        let face = solid.face(face)?;
        Some(Self {
            name: face.name(),
            origin: face.origin(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct ShellRecipe {
    pub feature: FeatureId,
    pub before: Arc<FeatureResult>,
    pub thickness: f64,
    pub side: caditor_kernel::ShellSide,
    pub open: Vec<NamedFace>,
    pub faces: Vec<(NamedFace, f64)>,
}

impl ShellRecipe {
    fn of(feature: FeatureId, before: Arc<FeatureResult>, open: &[FaceId], walls: &Walls) -> Self {
        let solid = before.solid().map(|result| &result.solid);
        let named = |face: FaceId| solid.and_then(|solid| NamedFace::of(solid, face));
        Self {
            feature,
            open: open.iter().filter_map(|face| named(*face)).collect(),
            faces: walls
                .faces
                .iter()
                .filter_map(|(face, thickness)| Some((named(*face)?, *thickness)))
                .collect(),
            thickness: walls.thickness,
            side: walls.side,
            before,
        }
    }

    pub fn before(&self) -> Option<&Solid> {
        self.before.solid().map(|result| &result.solid)
    }
}

impl PartialEq for ShellRecipe {
    fn eq(&self, other: &Self) -> bool {
        self.feature == other.feature
            && self.thickness == other.thickness
            && self.side == other.side
            && self.open == other.open
            && self.faces == other.faces
            && (Arc::ptr_eq(&self.before, &other.before) || self.before == other.before)
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
    fn error(&self, cause: ShellCause) -> Failure {
        Failure::from(FeatureError::new(cause).fixing(FixTarget::Feature(self.feature.id())))
    }

    fn length(&self, expression: &Expression, face: Option<&str>) -> Result<f64, Failure> {
        let value = expression
            .evaluate_as(Dimension::LENGTH, &|id| self.inputs.parameters.value(id))
            .map_err(|error| {
                let of_face = face.map(|face| format!(" of {face}")).unwrap_or_default();
                Failure::from(FeatureError::unevaluable(
                    self.feature.id(),
                    format!("thickness{of_face}"),
                    Expected::Length(2.0),
                    error,
                ))
            })?;
        if value > 0.0 {
            Ok(value)
        } else {
            Err(self.error(ShellCause::NotPositive {
                face: face.map(str::to_owned),
            }))
        }
    }

    fn walls(&self, solid: &Solid, thickness: f64) -> Result<Walls, Failure> {
        let mut faces = BTreeMap::new();
        let resolutions = self.definition.wall_resolutions(solid);
        for (entry, resolution) in self.definition.faces.iter().zip(resolutions) {
            let described = describe_origin(self.inputs.document, entry.face.origin());
            let value = self.length(&entry.thickness, Some(&described))?;
            match resolution {
                Resolution::One(face) => {
                    faces.insert(face, value);
                }
                Resolution::Pieces(pieces) => {
                    faces.extend(pieces.into_iter().map(|face| (face, value)));
                }
                Resolution::Tied(_) => {
                    return Err(self.error(ShellCause::WallTied {
                        face: described,
                        body: self.body_name.clone(),
                    }));
                }
                Resolution::Missing => {
                    return Err(self.error(ShellCause::WallGone {
                        face: described,
                        body: self.body_name.clone(),
                    }));
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

    fn failure(&self, solid: &Solid, open: &[FaceId], error: &ShellError) -> Failure {
        let body = self.body_name.clone();
        let face = |face: &FaceId| self.describe_face(solid, *face);
        let edge = |edge: &EdgeId| describe_edge(self.inputs.document, solid, *edge);
        match error {
            ShellError::Cancelled(_) => Failure::Cancelled,
            ShellError::InvalidThickness => self.error(ShellCause::ThicknessTooSmall),
            ShellError::MissingFace(_) => self.error(ShellCause::FaceGone { body }),
            ShellError::InvalidFaceThickness(of) => {
                self.error(ShellCause::FaceThicknessTooSmall { face: face(of) })
            }
            ShellError::OpenedFaceThickness(of) => {
                self.error(ShellCause::OpenedFaceThickness { face: face(of) })
            }
            ShellError::UnsupportedFace(of) if open.contains(of) => {
                self.error(ShellCause::UnsupportedOpening { face: face(of) })
            }
            ShellError::CurvedOpening(of) => {
                self.error(ShellCause::CurvedOpening { face: face(of) })
            }
            ShellError::UnsupportedFace(of) => {
                self.error(ShellCause::UnsupportedWall { face: face(of) })
            }
            ShellError::UnsupportedEdge(of) => {
                self.error(ShellCause::UnsupportedEdge { edge: edge(of) })
            }
            ShellError::TooCurved(of) => self.error(ShellCause::TooCurved { face: face(of) }),
            ShellError::Corner(vertex) => self.error(ShellCause::Corner {
                body,
                corner: self.describe_corner(solid, *vertex),
            }),
            ShellError::EdgeCollapses(of) => {
                self.error(ShellCause::EdgeCollapses { edge: edge(of) })
            }
            ShellError::ClosesBesideClosing { face: of, beyond } => {
                self.error(ShellCause::ClosesBesideClosing {
                    face: face(of),
                    beyond: face(beyond),
                })
            }
            ShellError::Opening(of) => self.error(ShellCause::Opening { face: face(of) }),
            ShellError::Overhang { open, wall } => self.error(ShellCause::Overhang {
                open: face(open),
                wall: face(wall),
            }),
            ShellError::Walls { faces, edge: near } => self.error(ShellCause::Crossing {
                body,
                near: Near {
                    faces: self.describe_faces(solid, faces),
                    edge: near.as_ref().map(edge),
                },
            }),
            ShellError::TooThick => self.error(ShellCause::TooThick { body }),
            ShellError::Voids(_) => {
                log::warn!("{} could not be built: {error}", self.feature.name);
                self.error(ShellCause::Undivided { body })
            }
            ShellError::Boolean(_) => {
                log::warn!("{} could not be built: {error}", self.feature.name);
                self.error(ShellCause::Unbuilt { body })
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ShellCause {
    NotPositive { face: Option<String> },
    WallTied { face: String, body: String },
    WallGone { face: String, body: String },
    ThicknessTooSmall,
    FaceGone { body: String },
    FaceThicknessTooSmall { face: String },
    OpenedFaceThickness { face: String },
    UnsupportedOpening { face: String },
    CurvedOpening { face: String },
    UnsupportedWall { face: String },
    UnsupportedEdge { edge: String },
    TooCurved { face: String },
    Corner { body: String, corner: String },
    EdgeCollapses { edge: String },
    ClosesBesideClosing { face: String, beyond: String },
    Opening { face: String },
    Overhang { open: String, wall: String },
    Crossing { body: String, near: Near },
    TooThick { body: String },
    Undivided { body: String },
    Unbuilt { body: String },
}

impl ShellCause {
    pub(crate) fn reason(&self) -> String {
        match self {
            Self::NotPositive { face } => {
                format!(
                    "The thickness{} must be more than zero.",
                    Self::of_face(face)
                )
            }
            Self::WallTied { face, body } => format!(
                "{} given its own thickness now matches several separate faces of the body of \
                 {body}.",
                capitalized(face)
            ),
            Self::WallGone { face, body } => format!(
                "{} given its own thickness is no longer part of the body of {body}.",
                capitalized(face)
            ),
            Self::ThicknessTooSmall => "The thickness must be more than 0.000001 mm.".to_owned(),
            Self::FaceGone { body } => {
                format!("A face to open is no longer part of the body of {body}.")
            }
            Self::FaceThicknessTooSmall { face } => {
                format!("The thickness of {face} must be more than 0.000001 mm.")
            }
            Self::OpenedFaceThickness { face } => format!(
                "{} is open, so it has no wall to give its own thickness.",
                capitalized(face)
            ),
            Self::UnsupportedOpening { face } => format!(
                "{} is swept from a spline or curve, and only flat faces and cylinder, cone, \
                 sphere or torus faces can be opened.",
                capitalized(face)
            ),
            Self::CurvedOpening { face } => format!(
                "{} is curved, so it can open only on the outside of the body and where sharp \
                 edges part it from the faces left closed.",
                capitalized(face)
            ),
            Self::UnsupportedWall { face } => format!(
                "The walls cannot follow {face}: faces swept from splines cannot be offset yet."
            ),
            Self::UnsupportedEdge { edge } => format!("The walls cannot follow {edge}."),
            Self::TooCurved { face } => format!(
                "{} curves more tightly than the thickness, so its wall would vanish.",
                capitalized(face)
            ),
            Self::Corner { body, corner } => {
                format!("The walls of the body of {body} cannot meet at {corner}.")
            }
            Self::EdgeCollapses { edge } => {
                format!("The wall along {edge} shrinks to nothing at this thickness.")
            }
            Self::ClosesBesideClosing { face, beyond } => format!(
                "{} and {beyond} beside it both close up at this thickness, so their walls have \
                 nowhere to meet.",
                capitalized(face)
            ),
            Self::Opening { face } => {
                format!("The opening in {face} could not be cut at this thickness.")
            }
            Self::Overhang { open, wall } => format!(
                "{} leans over the opening in {open}, so cutting the opening at this thickness \
                 would leave its wall thinner than the rest.",
                capitalized(wall)
            ),
            Self::Crossing { body, near } => format!(
                "The walls of the body of {body} would cross each other{near} at this thickness."
            ),
            Self::TooThick { body } => {
                format!("The thickness is too large for the body of {body}.")
            }
            Self::Undivided { body } => format!(
                "The body of {body} could not be divided into its outside and its hollows, so it \
                 cannot be shelled."
            ),
            Self::Unbuilt { body } => {
                format!("The shell could not be built on the body of {body}.")
            }
        }
    }

    pub(crate) fn remedy(&self) -> String {
        match self {
            Self::NotPositive { face } => {
                format!("Enter a thickness{} above zero.", Self::of_face(face))
            }
            Self::WallTied { .. } | Self::WallGone { .. } => {
                "Choose the face again, or remove its own thickness from the shell.".to_owned()
            }
            Self::ThicknessTooSmall => "Enter a larger thickness.".to_owned(),
            Self::FaceGone { .. } => {
                "Choose the faces again, or undo the change that removed it.".to_owned()
            }
            Self::FaceThicknessTooSmall { .. } => {
                "Enter a larger thickness for that face.".to_owned()
            }
            Self::OpenedFaceThickness { .. } => {
                "Remove its own thickness, or close the face again.".to_owned()
            }
            Self::UnsupportedOpening { .. } => "Leave this face out.".to_owned(),
            Self::CurvedOpening { .. } => {
                "Open the faces it meets smoothly as well, or open a flat face instead.".to_owned()
            }
            Self::UnsupportedWall { .. } => {
                "Shell the body before adding those faces, or open that face.".to_owned()
            }
            Self::UnsupportedEdge { .. } => {
                "Try a different thickness, or shell the body before the feature that made this \
                 edge."
                    .to_owned()
            }
            Self::TooCurved { .. } => {
                "Enter a thickness below the smallest radius of that face.".to_owned()
            }
            Self::Corner { .. } => {
                "Change the thickness, or open one of the faces at that corner.".to_owned()
            }
            Self::EdgeCollapses { .. }
            | Self::ClosesBesideClosing { .. }
            | Self::Overhang { .. }
            | Self::Crossing { .. }
            | Self::TooThick { .. } => "Enter a smaller thickness.".to_owned(),
            Self::Opening { .. } => {
                "Enter a smaller thickness, or leave this face closed.".to_owned()
            }
            Self::Undivided { .. } => {
                "Change the feature that made the body slightly, or shell it before that \
                 feature."
                    .to_owned()
            }
            Self::Unbuilt { .. } => {
                "Change the thickness slightly, or open different faces.".to_owned()
            }
        }
    }

    fn of_face(face: &Option<String>) -> String {
        face.as_ref()
            .map(|face| format!(" of {face}"))
            .unwrap_or_default()
    }
}

impl From<ShellCause> for Cause {
    fn from(cause: ShellCause) -> Self {
        Self::Shell(cause)
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
    let Some((_, before)) = inputs.bodies.get(&definition.body) else {
        return Err(inputs.missing_body(definition.body));
    };
    let Some(solid) = inputs.body(definition.body) else {
        return Err(inputs.missing_body(definition.body));
    };
    let open = definition.resolve(solid).map_err(|unresolved| {
        Failure::from(
            FeatureError::new(Unmatched {
                chosen: Chosen::FacesToOpen,
                unresolved,
                body: context.body_name.clone(),
            })
            .fixing(FixTarget::Feature(feature.id())),
        )
    })?;
    let walls = context.walls(solid, thickness)?;
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let result = shell_walls(solid, &open, &walls, feature.id().raw())
        .map_err(|error| context.failure(solid, &open, &error))?;
    let recipe = ShellRecipe::of(feature.id(), Arc::clone(before), &open, &walls);
    Ok(FeatureResult::Solid(
        SolidResult::new(definition.body, result).shelling(recipe),
    ))
}
