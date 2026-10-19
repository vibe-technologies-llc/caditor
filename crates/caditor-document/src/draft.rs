use std::{borrow::Cow, collections::BTreeSet};

use caditor_expression::{Dimension, Expression, ParameterId};
use caditor_geometry::{Plane, Point3, Vector3};
use caditor_kernel::{
    DraftError, DraftedFace, EdgeId, Face, FaceId, FaceName, FaceReference, FaceSplitError,
    LINEAR_RESOLUTION, MoveError, STEEPEST_DRAFT, Solid, SplitPiece, Surface, check_draftable,
    draft_faces, split_faces, tangent_faces,
};

use crate::{
    attachment::{FaceAttachment, face_plane},
    datum::{PlaneReference, PrincipalPlane, Resolver, capitalized},
    describe::{describe_corner, describe_edge, describe_face, describe_faces, edge_faces},
    document::{Feature, FeatureId},
    failure::{Cause, Chosen, Expected, Near, Unmatched},
    origins,
    pieces::{Resolution, Unresolved, pieces_of_one_face, tally},
    recompute::{
        CancelToken, Failure, FeatureError, FeatureResult, FixTarget, Inputs, MissingInput,
    },
    solid::SolidResult,
    split::{HalfSpaceError, half_space_solid},
};

const BOUNDARY_SAMPLES: usize = 8;
const SQUARE_TO_WALL: f64 = 1e-6;
const ACROSS_WALL: f64 = 0.5;

#[derive(Debug, Clone, PartialEq)]
pub struct FaceAngle {
    pub face: FaceReference,
    pub angle: Expression,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    pub body: FeatureId,
    pub faces: Vec<FaceReference>,
    pub neutral: PlaneReference,
    pub angle: Expression,
    pub flipped: bool,
    pub tangent: bool,
    pub beyond: Option<Expression>,
    pub own: Vec<FaceAngle>,
}

impl Draft {
    pub fn expressions(&self) -> Vec<&Expression> {
        std::iter::once(&self.angle)
            .chain(self.beyond.iter())
            .chain(self.own.iter().map(|own| &own.angle))
            .collect()
    }

    pub fn expressions_mut(&mut self) -> Vec<&mut Expression> {
        std::iter::once(&mut self.angle)
            .chain(self.beyond.iter_mut())
            .chain(self.own.iter_mut().map(|own| &mut own.angle))
            .collect()
    }

    pub fn heap_size(&self) -> usize {
        size_of_val(self.faces.as_slice())
            + self
                .faces
                .iter()
                .map(FaceReference::heap_size)
                .sum::<usize>()
            + size_of_val(self.own.as_slice())
            + self
                .own
                .iter()
                .map(|own| own.face.heap_size())
                .sum::<usize>()
            + self
                .expressions()
                .into_iter()
                .map(Expression::heap_size)
                .sum::<usize>()
            + self.neutral.heap_size()
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

    pub fn features(&self) -> BTreeSet<FeatureId> {
        let mut used = BTreeSet::from([self.body]);
        used.extend(self.neutral.datum());
        used.extend(self.neutral.frame());
        used.extend(self.neutral.body());
        used
    }

    pub fn origin_features(&self) -> BTreeSet<FeatureId> {
        let mut origins = self.neutral.origin_features();
        origins.extend(self.faces.iter().flat_map(origins::of_face));
        origins.extend(self.own.iter().flat_map(|own| origins::of_face(&own.face)));
        origins
    }

    pub fn resolutions(&self, solid: &Solid) -> Vec<Resolution<FaceId>> {
        self.faces
            .iter()
            .map(|reference| resolution(solid, reference))
            .collect()
    }

    pub fn own_resolutions(&self, solid: &Solid) -> Vec<Resolution<FaceId>> {
        self.own
            .iter()
            .map(|own| resolution(solid, &own.face))
            .collect()
    }

    pub fn resolve(&self, solid: &Solid) -> Result<Vec<FaceId>, Unresolved> {
        tally(self.resolutions(solid))
    }

    pub fn splits(&self) -> bool {
        self.beyond.is_some()
    }
}

fn resolution(solid: &Solid, reference: &FaceReference) -> Resolution<FaceId> {
    Resolution::of(reference.resolve(solid), |pieces| {
        pieces_of_one_face(solid, pieces)
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Tilted {
    face: FaceId,
    own: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Along,
    Beyond,
}

fn boundary_points(solid: &Solid, face: FaceId) -> Vec<Point3> {
    let Some(definition) = solid.face(face) else {
        return Vec::new();
    };
    let edges: BTreeSet<EdgeId> = definition
        .loops()
        .iter()
        .filter_map(|id| solid.face_loop(*id))
        .flat_map(|face_loop| face_loop.coedges())
        .filter_map(|coedge| solid.coedge(*coedge))
        .map(|coedge| coedge.edge())
        .collect();
    edges
        .into_iter()
        .filter_map(|edge| solid.edge(edge))
        .flat_map(|edge| {
            edge.interval()
                .split(BOUNDARY_SAMPLES)
                .map(|parameter| edge.curve().point(parameter))
                .collect::<Vec<_>>()
        })
        .collect()
}

fn boundary_heights(solid: &Solid, face: FaceId, plane: &Plane) -> Vec<f64> {
    boundary_points(solid, face)
        .into_iter()
        .map(|point| plane.signed_distance(point))
        .collect()
}

fn straddles(solid: &Solid, face: FaceId, plane: &Plane) -> bool {
    let heights = boundary_heights(solid, face, plane);
    heights.iter().any(|height| *height > LINEAR_RESOLUTION)
        && heights.iter().any(|height| *height < -LINEAR_RESOLUTION)
}

fn side_of(solid: &Solid, face: FaceId, plane: &Plane) -> Side {
    let farthest = boundary_heights(solid, face, plane)
        .into_iter()
        .max_by(|a, b| a.abs().total_cmp(&b.abs()))
        .unwrap_or(0.0);
    if farthest < 0.0 {
        Side::Beyond
    } else {
        Side::Along
    }
}

struct Original {
    name: FaceName,
    surface: Surface,
    split: bool,
    own: Option<f64>,
}

impl Original {
    fn became(&self, face: &Face, raw: u64) -> bool {
        let named = if self.split {
            [SplitPiece::Inside, SplitPiece::Outside]
                .into_iter()
                .any(|piece| face.name() == FaceName::split(raw, self.name, piece))
        } else {
            face.name() == self.name
        };
        named && face.surface().same_surface(&self.surface).is_some()
    }
}

fn faces_after_split(split: &Solid, raw: u64, chosen: &[Original]) -> Vec<Tilted> {
    split
        .faces()
        .filter_map(|(id, face)| {
            let original = chosen.iter().find(|original| original.became(face, raw))?;
            Some(Tilted {
                face: id,
                own: original.own,
            })
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Angled {
    Along,
    Beyond,
    Own,
}

struct Context<'a> {
    feature: &'a Feature,
    definition: &'a Draft,
    inputs: &'a Inputs<'a>,
    body_name: String,
}

impl Context<'_> {
    fn error(&self, cause: DraftCause) -> Failure {
        Failure::from(FeatureError::new(cause).fixing(FixTarget::Feature(self.feature.id())))
    }

    fn angle(&self, expression: &Expression, angled: Angled) -> Result<f64, Failure> {
        let what = match angled {
            Angled::Along => "draft angle",
            Angled::Beyond => "angle beyond the neutral plane",
            Angled::Own => "own draft angle of a face",
        };
        let degrees = expression
            .evaluate_as(Dimension::ANGLE, &|id| self.inputs.parameters.value(id))
            .map_err(|error| {
                Failure::from(FeatureError::unevaluable(
                    self.feature.id(),
                    what,
                    Expected::Angle(3.0),
                    error,
                ))
            })?;
        let radians = degrees.to_radians();
        if !radians.is_finite() || radians.abs() > STEEPEST_DRAFT {
            return Err(self.error(DraftCause::Steep { angled }));
        }
        Ok(radians)
    }

    fn neutral(&self) -> Result<Plane, Failure> {
        let resolver = Resolver {
            feature: self.feature,
            inputs: self.inputs,
        };
        let plane = resolver.plane(&self.definition.neutral)?;
        let pull = if self.definition.flipped {
            -plane.normal()
        } else {
            plane.normal()
        };
        Plane::new(plane.origin(), pull).ok_or_else(|| self.error(DraftCause::Unsplit))
    }

    fn check_draftable(
        &self,
        solid: &Solid,
        faces: &[Tilted],
        picked: &[FaceId],
        neutral: &Plane,
    ) -> Result<(), Failure> {
        for face in faces.iter().map(|chosen| chosen.face) {
            let chained = !picked.contains(&face);
            match check_draftable(solid, face, neutral) {
                Ok(()) => {}
                Err(DraftError::NotDraftable(_)) => {
                    return Err(self.error(DraftCause::NotDraftable {
                        face: self.describe(solid, face),
                        chained,
                    }));
                }
                Err(DraftError::OffPull(_)) => {
                    return Err(self.error(DraftCause::OffPull {
                        face: self.describe(solid, face),
                        chained,
                    }));
                }
                Err(error) => return Err(self.failure(solid, &error)),
            }
        }
        Ok(())
    }

    fn own_angles(&self, solid: &Solid) -> Result<Vec<Tilted>, Failure> {
        let faces = tally(self.definition.own_resolutions(solid)).map_err(|unresolved| {
            Failure::from(
                FeatureError::new(Unmatched {
                    chosen: Chosen::FacesWithOwnAngle,
                    unresolved,
                    body: self.body_name.clone(),
                })
                .fixing(FixTarget::Feature(self.feature.id())),
            )
        })?;
        let mut chosen = Vec::with_capacity(faces.len());
        for (resolution, own) in self
            .definition
            .own_resolutions(solid)
            .iter()
            .zip(&self.definition.own)
        {
            let angle = self.angle(&own.angle, Angled::Own)?;
            chosen.extend(resolution.found().iter().map(|face| Tilted {
                face: *face,
                own: Some(angle),
            }));
        }
        Ok(chosen)
    }

    fn describe(&self, solid: &Solid, face: FaceId) -> String {
        describe_face(self.inputs.document, solid, face)
    }

    fn failure(&self, solid: &Solid, error: &DraftError) -> Failure {
        let body = self.body_name.clone();
        let face = |face: &FaceId| self.describe(solid, *face);
        match error {
            DraftError::NothingToDraft => Failure::Waiting(MissingInput::FacesToDraft {
                body: self.definition.body,
            }),
            DraftError::MissingFace(_) => self.error(DraftCause::FaceGone { body }),
            DraftError::TooSteep(_) => self.error(DraftCause::Steep {
                angled: Angled::Along,
            }),
            DraftError::NotDraftable(of) => self.error(DraftCause::NotDraftable {
                face: face(of),
                chained: false,
            }),
            DraftError::OffPull(of) => self.error(DraftCause::OffPull {
                face: face(of),
                chained: false,
            }),
            DraftError::NarrowsToPoint(of) => {
                self.error(DraftCause::NarrowsToPoint { face: face(of) })
            }
            DraftError::AlongNeutral(of) => self.error(DraftCause::AlongNeutral { face: face(of) }),
            DraftError::AlreadyDrafted | DraftError::Move(MoveError::Unmoved) => {
                self.error(DraftCause::AlreadyDrafted)
            }
            DraftError::Move(error) => self.move_failure(solid, error),
        }
    }

    fn move_failure(&self, solid: &Solid, error: &MoveError) -> Failure {
        let document = self.inputs.document;
        let body = self.body_name.clone();
        let face = |face: &FaceId| self.describe(solid, *face);
        let edge = |edge: &EdgeId| describe_edge(document, solid, *edge);
        match error {
            MoveError::Cancelled(_) => Failure::Cancelled,
            MoveError::Unmoved => self.error(DraftCause::AlreadyDrafted),
            MoveError::NothingToMove => Failure::Waiting(MissingInput::FacesToDraft {
                body: self.definition.body,
            }),
            MoveError::MissingFace(_) => self.error(DraftCause::FaceGone { body }),
            MoveError::UnsupportedFace(of) => self.error(DraftCause::NotDraftable {
                face: face(of),
                chained: false,
            }),
            MoveError::UnsupportedEdge(of) => {
                self.error(DraftCause::UnsupportedEdge { edge: edge(of) })
            }
            MoveError::Vanishes(of) => self.error(DraftCause::Vanishes { face: face(of) }),
            MoveError::Corner(vertex) => self.error(DraftCause::Corner {
                body,
                corner: describe_corner(document, solid, *vertex),
            }),
            MoveError::EdgeCollapses(of) => {
                self.error(DraftCause::EdgeCollapses { edge: edge(of) })
            }
            MoveError::Invalid { faces, edge: near } => self.error(DraftCause::Invalid {
                body,
                near: Near {
                    faces: describe_faces(document, solid, faces),
                    edge: near.as_ref().map(edge),
                },
            }),
            MoveError::Crosses { first, second } => self.error(DraftCause::Crosses {
                first: face(first),
                second: face(second),
            }),
        }
    }

    fn split_failure(&self, error: &FaceSplitError) -> Failure {
        match error {
            FaceSplitError::Cancelled(_) => Failure::Cancelled,
            FaceSplitError::NoFaces | FaceSplitError::Undivided | FaceSplitError::Boolean(_) => {
                self.error(DraftCause::Unsplit)
            }
        }
    }

    fn half_space_failure(&self, error: &HalfSpaceError) -> Failure {
        log::warn!(
            "{} could not split at its neutral plane: {error}",
            self.feature.name
        );
        self.error(DraftCause::Unsplit)
    }

    fn split_at_neutral<'s>(
        &self,
        solid: &'s Solid,
        faces: Vec<Tilted>,
        neutral: &Plane,
    ) -> Result<(Cow<'s, Solid>, Vec<Tilted>), Failure> {
        let crossing: Vec<FaceId> = faces
            .iter()
            .map(|chosen| chosen.face)
            .filter(|face| straddles(solid, *face, neutral))
            .collect();
        if crossing.is_empty() {
            return Ok((Cow::Borrowed(solid), faces));
        }
        let raw = self.feature.id().raw();
        let chosen: Vec<Original> = faces
            .iter()
            .filter_map(|chosen| {
                let definition = solid.face(chosen.face)?;
                Some(Original {
                    name: definition.name(),
                    surface: definition.surface().clone(),
                    split: crossing.contains(&chosen.face),
                    own: chosen.own,
                })
            })
            .collect();
        let tool = half_space_solid(solid, neutral, false, raw)
            .map_err(|error| self.half_space_failure(&error))?;
        let split = split_faces(solid, &crossing, &tool, raw)
            .map_err(|error| self.split_failure(&error))?;
        let faces = faces_after_split(&split, raw, &chosen);
        Ok((Cow::Owned(split), faces))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum DraftCause {
    Steep { angled: Angled },
    FaceGone { body: String },
    NotDraftable { face: String, chained: bool },
    OffPull { face: String, chained: bool },
    NarrowsToPoint { face: String },
    AlongNeutral { face: String },
    AlreadyDrafted,
    Unsplit,
    UnsupportedEdge { edge: String },
    Vanishes { face: String },
    Corner { body: String, corner: String },
    EdgeCollapses { edge: String },
    Invalid { body: String, near: Near },
    Crosses { first: String, second: String },
}

impl DraftCause {
    pub(crate) fn reason(&self) -> String {
        match self {
            Self::Steep {
                angled: Angled::Along,
            } => "The draft angle is steeper than 89 deg.".to_owned(),
            Self::Steep {
                angled: Angled::Beyond,
            } => "The angle beyond the neutral plane is steeper than 89 deg.".to_owned(),
            Self::Steep {
                angled: Angled::Own,
            } => "The own draft angle of a face is steeper than 89 deg.".to_owned(),
            Self::FaceGone { body } => {
                format!("A face to draft is no longer part of the body of {body}.")
            }
            Self::NotDraftable { face, chained } => format!(
                "{} is curved in a way that cannot be drafted{}; only flat faces and round faces \
                 about the pull direction can be.",
                capitalized(face),
                joining(*chained)
            ),
            Self::OffPull { face, chained } => format!(
                "{} is round about an axis that does not run along the pull direction{}, so it \
                 cannot be drafted.",
                capitalized(face),
                joining(*chained)
            ),
            Self::NarrowsToPoint { face } => format!(
                "{} would narrow to a point with this draft.",
                capitalized(face)
            ),
            Self::AlongNeutral { face } => format!(
                "{} is parallel to the neutral plane, so it has no line to tilt about.",
                capitalized(face)
            ),
            Self::AlreadyDrafted => {
                "Every face already makes the draft angle with the pull direction.".to_owned()
            }
            Self::Unsplit => "The faces could not be split at the neutral plane.".to_owned(),
            Self::UnsupportedEdge { edge } => {
                format!("The faces beside the drafted ones cannot follow {edge}.")
            }
            Self::Vanishes { face } => {
                format!("{} would vanish with this draft.", capitalized(face))
            }
            Self::Corner { body, corner } => {
                format!("The faces of the body of {body} cannot meet at {corner}.")
            }
            Self::EdgeCollapses { edge } => {
                format!("The face along {edge} would shrink to nothing with this draft.")
            }
            Self::Invalid { body, near } => format!(
                "The body of {body} would not stay valid{near} with this draft, as a drafted face \
                 would pass through another part of it."
            ),
            Self::Crosses { first, second } => {
                format!("{first} and {second} would cross each other with this draft.")
            }
        }
    }

    pub(crate) fn remedy(&self) -> String {
        match self {
            Self::Steep { .. } => "Enter an angle between -89 deg and 89 deg.".to_owned(),
            Self::FaceGone { .. } => {
                "Choose the faces again, or undo the change that removed it.".to_owned()
            }
            Self::NotDraftable { chained: true, .. } | Self::OffPull { chained: true, .. } => {
                "Turn off Follow tangent faces, or draft before rounding the edges between these \
                 faces."
                    .to_owned()
            }
            Self::NotDraftable { chained: false, .. } => "Leave this face out.".to_owned(),
            Self::OffPull { chained: false, .. } => {
                "Leave this face out, or choose a neutral plane square to its axis.".to_owned()
            }
            Self::AlongNeutral { .. } => {
                "Leave this face out, or choose a neutral plane that crosses it.".to_owned()
            }
            Self::AlreadyDrafted => "Enter another angle, or choose other faces.".to_owned(),
            Self::Unsplit => {
                "Draft on one side only, or choose a neutral plane through the faces.".to_owned()
            }
            Self::UnsupportedEdge { .. } | Self::Corner { .. } => {
                "Draft before rounding the edges beside these faces, or draft the faces beside \
                 them too."
                    .to_owned()
            }
            Self::Vanishes { .. }
            | Self::NarrowsToPoint { .. }
            | Self::EdgeCollapses { .. }
            | Self::Invalid { .. }
            | Self::Crosses { .. } => {
                "Enter a smaller angle, or move the neutral plane closer to the faces.".to_owned()
            }
        }
    }
}

fn joining(chained: bool) -> &'static str {
    if chained {
        ", and it joins the chosen faces smoothly"
    } else {
        ""
    }
}

impl From<DraftCause> for Cause {
    fn from(cause: DraftCause) -> Self {
        Self::Draft(cause)
    }
}

fn drafted(
    solid: &Solid,
    faces: &[Tilted],
    neutral: &Plane,
    angle: f64,
    beyond: Option<f64>,
) -> Vec<DraftedFace> {
    faces
        .iter()
        .map(|chosen| DraftedFace {
            face: chosen.face,
            angle: match (beyond, side_of(solid, chosen.face, neutral)) {
                (Some(beyond), Side::Beyond) => -beyond,
                _ => chosen.own.unwrap_or(angle),
            },
        })
        .collect()
}

pub(crate) fn evaluate(
    feature: &Feature,
    definition: &Draft,
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
                chosen: Chosen::FacesToDraft,
                unresolved,
                body: context.body_name.clone(),
            })
            .fixing(FixTarget::Feature(feature.id())),
        )
    })?;
    let owned = context.own_angles(solid)?;
    if faces.is_empty() && owned.is_empty() {
        return Err(Failure::Waiting(MissingInput::FacesToDraft {
            body: definition.body,
        }));
    }
    let angle = context.angle(&definition.angle, Angled::Along)?;
    let beyond = definition
        .beyond
        .as_ref()
        .map(|expression| context.angle(expression, Angled::Beyond))
        .transpose()?;
    let neutral = context.neutral()?;
    let mut picked = faces;
    let common = if definition.tangent {
        tangent_faces(solid, &picked)
    } else {
        picked.clone()
    };
    let faces: Vec<Tilted> = common
        .into_iter()
        .filter(|face| owned.iter().all(|own| own.face != *face))
        .map(|face| Tilted { face, own: None })
        .chain(owned.iter().copied())
        .collect();
    picked.extend(owned.iter().map(|own| own.face));
    context.check_draftable(solid, &faces, &picked, &neutral)?;
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let (solid, faces) = if beyond.is_some() {
        context.split_at_neutral(solid, faces, &neutral)?
    } else {
        (Cow::Borrowed(solid), faces)
    };
    let drafted = drafted(&solid, &faces, &neutral, angle, beyond);
    let result =
        draft_faces(&solid, &drafted, &neutral).map_err(|error| context.failure(&solid, &error))?;
    Ok(FeatureResult::Solid(SolidResult::new(
        definition.body,
        result,
    )))
}

pub fn flipped_toward(plane: &Plane, solid: &Solid, faces: &[FaceId]) -> bool {
    let heights: Vec<f64> = faces
        .iter()
        .flat_map(|face| boundary_heights(solid, *face, plane))
        .collect();
    let mean = heights.iter().sum::<f64>() / heights.len().max(1) as f64;
    mean < 0.0
}

fn neighbours(solid: &Solid, face: FaceId) -> Vec<FaceId> {
    let Some(definition) = solid.face(face) else {
        return Vec::new();
    };
    let edges: BTreeSet<EdgeId> = definition
        .loops()
        .iter()
        .filter_map(|id| solid.face_loop(*id))
        .flat_map(|face_loop| face_loop.coedges())
        .filter_map(|coedge| solid.coedge(*coedge))
        .map(|coedge| coedge.edge())
        .collect();
    edges
        .into_iter()
        .flat_map(|edge| edge_faces(solid, edge))
        .filter(|other| *other != face)
        .collect()
}

fn floor_beside(solid: &Solid, face: FaceId) -> Option<FaceId> {
    let normal = face_plane(solid, face)?.normal();
    neighbours(solid, face)
        .into_iter()
        .filter_map(|other| Some((other, face_plane(solid, other)?.normal())))
        .filter(|(_, beside)| beside.dot(normal).abs() <= SQUARE_TO_WALL)
        .min_by(|(_, a), (_, b)| a.z.total_cmp(&b.z))
        .map(|(other, _)| other)
}

fn round_axis(solid: &Solid, face: FaceId) -> Option<Vector3> {
    match solid.face(face)?.surface() {
        Surface::Cylinder(cylinder) => Some(cylinder.frame().normal()),
        Surface::Cone(cone) => Some(cone.frame().normal()),
        _ => None,
    }
}

fn floor_of_round(solid: &Solid, face: FaceId) -> Option<FaceId> {
    let axis = round_axis(solid, face)?;
    let points = boundary_points(solid, face);
    let middle =
        points.iter().fold(Point3::ZERO, |sum, point| sum + *point) / points.len().max(1) as f64;
    neighbours(solid, face)
        .into_iter()
        .filter_map(|other| Some((other, face_plane(solid, other)?)))
        .filter(|(_, end)| end.normal().dot(axis).abs() >= 1.0 - SQUARE_TO_WALL)
        .min_by(|(_, a), (_, b)| {
            let facing_away = |end: &Plane| end.signed_distance(middle) <= 0.0;
            facing_away(a)
                .cmp(&facing_away(b))
                .then(a.normal().z.total_cmp(&b.normal().z))
        })
        .map(|(other, _)| other)
}

pub fn default_neutral(body: FeatureId, solid: &Solid, faces: &[FaceId]) -> (PlaneReference, bool) {
    let first_flat = faces
        .iter()
        .find_map(|face| Some((*face, face_plane(solid, *face)?)));
    let first_axis = faces.iter().find_map(|face| round_axis(solid, *face));
    let floor = match first_flat {
        Some((face, _)) => floor_beside(solid, face),
        None => faces.iter().find_map(|face| floor_of_round(solid, *face)),
    };
    let on_floor = floor.and_then(|floor| FaceAttachment::capture(body, solid, floor));
    let (reference, plane) = match on_floor {
        Some((attachment, plane)) => (PlaneReference::Face(attachment), plane),
        None => {
            let principal = [PrincipalPlane::Xy, PrincipalPlane::Xz, PrincipalPlane::Yz]
                .into_iter()
                .find(|principal| {
                    let normal = principal.plane().normal();
                    match (first_flat, first_axis) {
                        (Some((_, wall)), _) => normal.dot(wall.normal()).abs() < ACROSS_WALL,
                        (None, Some(axis)) => normal.dot(axis).abs() > ACROSS_WALL,
                        (None, None) => true,
                    }
                })
                .unwrap_or(PrincipalPlane::Xy);
            (PlaneReference::Principal(principal), principal.plane())
        }
    };
    (reference, flipped_toward(&plane, solid, faces))
}
