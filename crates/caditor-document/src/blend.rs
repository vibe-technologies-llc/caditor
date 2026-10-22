use std::collections::{BTreeMap, BTreeSet};

use caditor_expression::{Dimension, Expression, ParameterId};
use caditor_kernel::{
    BlendError, BlendShape, EdgeConvexity, EdgeId, EdgeNaming, EdgeReference, FaceId, FaceName,
    FaceOrigin, FaceReference, Solid, VertexId, blend, edge_convexity, faces_around_vertices,
    meets_back_to_back,
};

use crate::{
    describe::{describe_edge, edge_faces, origin_feature},
    document::{Document, Feature, FeatureId},
    failure::{Cause, Chosen, Expected, Unmatched},
    origins,
    pieces::{Resolution, Unresolved, pieces_of_one_face, tally},
    recompute::{
        CancelToken, Failure, FeatureError, FeatureResult, FixTarget, Inputs, MissingInput,
    },
    solid::SolidResult,
    split_edges::{Followed, SplitEdges},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BlendKind {
    Fillet,
    Chamfer,
}

impl BlendKind {
    pub fn title(self) -> &'static str {
        match self {
            Self::Fillet => "Fillet",
            Self::Chamfer => "Chamfer",
        }
    }

    pub fn noun(self) -> &'static str {
        match self {
            Self::Fillet => "fillet",
            Self::Chamfer => "chamfer",
        }
    }

    pub fn size_name(self) -> &'static str {
        match self {
            Self::Fillet => "radius",
            Self::Chamfer => "distance",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub enum ChamferForm {
    #[default]
    Equal,
    TwoDistances {
        second: Expression,
    },
    DistanceAngle {
        angle: Expression,
    },
}

impl ChamferForm {
    pub fn title(&self) -> &'static str {
        match self {
            Self::Equal => "Equal",
            Self::TwoDistances { .. } => "Two distances",
            Self::DistanceAngle { .. } => "Distance and angle",
        }
    }

    pub fn is_equal(&self) -> bool {
        matches!(self, Self::Equal)
    }

    fn expression(&self) -> Option<&Expression> {
        match self {
            Self::Equal => None,
            Self::TwoDistances { second } => Some(second),
            Self::DistanceAngle { angle } => Some(angle),
        }
    }

    pub fn expression_mut(&mut self) -> Option<&mut Expression> {
        match self {
            Self::Equal => None,
            Self::TwoDistances { second } => Some(second),
            Self::DistanceAngle { angle } => Some(angle),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeRule {
    Convex,
    Concave,
    MadeBy(FeatureId),
}

impl EdgeRule {
    pub fn edges(self, document: &Document, solid: &Solid) -> Vec<EdgeId> {
        let candidates = blendable(solid, solid.edges().map(|(id, _)| id));
        match self {
            Self::Convex => with_convexity(solid, candidates, EdgeConvexity::Convex),
            Self::Concave => with_convexity(solid, candidates, EdgeConvexity::Concave),
            Self::MadeBy(feature) => {
                let order = feature_order(document);
                candidates
                    .into_iter()
                    .filter(|edge| maker_of(solid, *edge, &order) == Some(feature))
                    .collect()
            }
        }
    }

    pub fn feature(self) -> Option<FeatureId> {
        match self {
            Self::MadeBy(feature) => Some(feature),
            Self::Convex | Self::Concave => None,
        }
    }
}

fn with_convexity(solid: &Solid, edges: Vec<EdgeId>, wanted: EdgeConvexity) -> Vec<EdgeId> {
    edges
        .into_iter()
        .filter(|edge| edge_convexity(solid, *edge) == Some(wanted))
        .collect()
}

fn feature_order(document: &Document) -> BTreeMap<FeatureId, usize> {
    document
        .features()
        .enumerate()
        .map(|(position, feature)| (feature.id(), position))
        .collect()
}

fn maker_of(solid: &Solid, edge: EdgeId, order: &BTreeMap<FeatureId, usize>) -> Option<FeatureId> {
    edge_faces(solid, edge)
        .into_iter()
        .filter_map(|face| solid.face(face)?.origin())
        .map(origin_feature)
        .filter_map(|feature| Some((*order.get(&feature)?, feature)))
        .max()
        .map(|(_, feature)| feature)
}

#[derive(Debug, Clone, PartialEq)]
pub enum EdgeGroup {
    Face(FaceReference),
    Body,
    Rule(EdgeRule),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupResolution {
    Face(Resolution<FaceId>),
    Body,
    Rule { rule: EdgeRule, edges: Vec<EdgeId> },
}

impl GroupResolution {
    pub fn edges(&self, solid: &Solid) -> Vec<EdgeId> {
        let faces: Vec<FaceId> = match self {
            Self::Body => return blendable(solid, solid.edges().map(|(id, _)| id)),
            Self::Rule { edges, .. } => return edges.clone(),
            Self::Face(resolution @ (Resolution::One(_) | Resolution::Pieces(_))) => {
                resolution.found().to_vec()
            }
            Self::Face(Resolution::Tied(_) | Resolution::Missing) => Vec::new(),
        };
        blendable(
            solid,
            faces.iter().flat_map(|face| face_boundary(solid, *face)),
        )
    }
}

pub fn face_boundary(solid: &Solid, face: FaceId) -> Vec<EdgeId> {
    solid
        .face(face)
        .into_iter()
        .flat_map(|face| face.loops())
        .filter_map(|id| solid.face_loop(*id))
        .flat_map(|face_loop| face_loop.coedges())
        .filter_map(|id| solid.coedge(*id))
        .map(|coedge| coedge.edge())
        .collect()
}

pub fn is_seam(solid: &Solid, edge: EdgeId) -> bool {
    edge_faces(solid, edge).len() == 1
}

fn blendable(solid: &Solid, edges: impl Iterator<Item = EdgeId>) -> Vec<EdgeId> {
    let mut seen = BTreeSet::new();
    edges
        .filter(|edge| {
            !is_seam(solid, *edge) && !meets_back_to_back(solid, *edge) && seen.insert(*edge)
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlendedEdge {
    pub reference: EdgeReference,
    pub ends: [BTreeMap<FaceName, Option<FaceOrigin>>; 2],
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlendRecipe {
    pub feature: FeatureId,
    pub shape: BlendShape,
    pub edges: Vec<BlendedEdge>,
}

impl BlendRecipe {
    fn of(feature: FeatureId, shape: BlendShape, solid: &Solid, edges: &[EdgeId]) -> Self {
        let naming = EdgeNaming::new(solid);
        let around = faces_around_vertices(solid);
        let origins: BTreeMap<FaceName, Option<FaceOrigin>> = solid
            .faces()
            .map(|(_, face)| (face.name(), face.origin()))
            .collect();
        let faces_at = |vertex: VertexId| {
            around
                .get(&vertex)
                .into_iter()
                .flatten()
                .map(|name| (*name, origins.get(name).copied().flatten()))
                .collect()
        };
        let edges = edges
            .iter()
            .filter_map(|edge| {
                let reference = EdgeReference::capture_in(&naming, *edge)?;
                let definition = solid.edge(*edge)?;
                Some(BlendedEdge {
                    reference,
                    ends: [faces_at(definition.start()), faces_at(definition.end())],
                })
            })
            .collect();
        Self {
            feature,
            shape,
            edges,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlendUnresolved {
    Edges(Unresolved),
    Faces(Unresolved),
    PartlyFollowed(usize),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Blend {
    pub kind: BlendKind,
    pub body: FeatureId,
    pub edges: Vec<EdgeReference>,
    pub groups: Vec<EdgeGroup>,
    pub size: Expression,
    pub form: ChamferForm,
    pub flipped: bool,
}

impl Blend {
    pub fn chamfer_form(&self) -> &ChamferForm {
        match self.kind {
            BlendKind::Fillet => &ChamferForm::Equal,
            BlendKind::Chamfer => &self.form,
        }
    }

    pub fn expressions(&self) -> impl Iterator<Item = &Expression> {
        std::iter::once(&self.size).chain(self.form.expression())
    }

    pub fn expressions_mut(&mut self) -> Vec<&mut Expression> {
        std::iter::once(&mut self.size)
            .chain(self.form.expression_mut())
            .collect()
    }

    pub fn heap_size(&self) -> usize {
        size_of_val(self.edges.as_slice())
            + size_of_val(self.groups.as_slice())
            + self.size.heap_size()
            + self.form.expression().map_or(0, Expression::heap_size)
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
        let mut used = BTreeSet::from([self.body]);
        used.extend(self.groups.iter().filter_map(|group| match group {
            EdgeGroup::Rule(rule) => rule.feature(),
            EdgeGroup::Face(_) | EdgeGroup::Body => None,
        }));
        used
    }

    pub fn origin_features(&self) -> BTreeSet<FeatureId> {
        self.edges
            .iter()
            .flat_map(origins::of_edge)
            .chain(self.groups.iter().flat_map(|group| match group {
                EdgeGroup::Face(face) => origins::of_face(face),
                EdgeGroup::Body | EdgeGroup::Rule(_) => BTreeSet::new(),
            }))
            .collect()
    }

    pub fn entry_count(&self) -> usize {
        self.edges.len() + self.groups.len()
    }

    pub fn remove_entry(&mut self, index: usize) {
        match index.checked_sub(self.edges.len()) {
            None => {
                self.edges.remove(index);
            }
            Some(group) if group < self.groups.len() => {
                self.groups.remove(group);
            }
            Some(_) => {}
        }
    }

    pub fn group_resolutions(&self, document: &Document, solid: &Solid) -> Vec<GroupResolution> {
        self.groups
            .iter()
            .map(|group| match group {
                EdgeGroup::Face(face) => {
                    GroupResolution::Face(Resolution::of(face.resolve(solid), |pieces| {
                        pieces_of_one_face(solid, pieces)
                    }))
                }
                EdgeGroup::Body => GroupResolution::Body,
                EdgeGroup::Rule(rule) => GroupResolution::Rule {
                    rule: *rule,
                    edges: rule.edges(document, solid),
                },
            })
            .collect()
    }

    pub fn entry_edges(&self, document: &Document, solid: &Solid) -> Vec<Vec<EdgeId>> {
        self.resolutions(document, solid)
            .iter()
            .map(|resolution| resolution.found().to_vec())
            .chain(
                self.group_resolutions(document, solid)
                    .iter()
                    .map(|resolution| resolution.edges(solid)),
            )
            .collect()
    }

    pub fn resolutions(&self, document: &Document, solid: &Solid) -> Vec<Resolution<EdgeId>> {
        self.followed(document, solid)
            .into_iter()
            .map(Followed::resolution)
            .collect()
    }

    pub(crate) fn followed(&self, document: &Document, solid: &Solid) -> Vec<Followed> {
        let split = SplitEdges::new(document, solid);
        self.edges
            .iter()
            .map(|reference| split.follow(reference))
            .collect()
    }

    pub fn resolve(
        &self,
        document: &Document,
        solid: &Solid,
    ) -> Result<Vec<EdgeId>, BlendUnresolved> {
        let groups = self.group_resolutions(document, solid);
        let faces = groups
            .iter()
            .filter_map(|resolution| match resolution {
                GroupResolution::Face(face) => Some(face.clone()),
                GroupResolution::Body | GroupResolution::Rule { .. } => None,
            })
            .collect();
        tally(faces).map_err(BlendUnresolved::Faces)?;
        let followed = self.followed(document, solid);
        let partly = followed
            .iter()
            .filter(|followed| matches!(followed, Followed::PartlyLost(_)))
            .count();
        if partly > 0 {
            return Err(BlendUnresolved::PartlyFollowed(partly));
        }
        let resolutions = followed.into_iter().map(Followed::resolution).collect();
        let mut edges = tally(resolutions).map_err(BlendUnresolved::Edges)?;
        edges.extend(groups.iter().flat_map(|group| group.edges(solid)));
        edges.sort_unstable();
        edges.dedup();
        Ok(edges)
    }
}

struct Context<'a> {
    feature: &'a Feature,
    definition: &'a Blend,
    inputs: &'a Inputs<'a>,
    body_name: String,
}

impl Context<'_> {
    fn earlier_blend_at(&self, solid: &Solid, vertex: VertexId) -> Option<String> {
        let document = self.inputs.document;
        solid
            .edges()
            .filter(|(_, edge)| edge.start() == vertex || edge.end() == vertex)
            .flat_map(|(edge, _)| edge_faces(solid, edge))
            .filter_map(|face| solid.face(face)?.origin())
            .find_map(|origin| match origin.original() {
                FaceOrigin::Fillet { feature } | FaceOrigin::Chamfer { feature }
                    if feature != self.feature.id().raw() =>
                {
                    document
                        .feature(FeatureId::from_raw(feature))
                        .map(|earlier| earlier.name.clone())
                }
                _ => None,
            })
    }

    fn error(&self, cause: BlendCause) -> Failure {
        Failure::from(FeatureError::new(cause).fixing(FixTarget::Feature(self.feature.id())))
    }

    fn value(
        &self,
        expression: &Expression,
        dimension: Dimension,
        what: &str,
    ) -> Result<f64, Failure> {
        let expected = if dimension == Dimension::ANGLE {
            Expected::Angle(45.0)
        } else {
            Expected::Length(2.0)
        };
        let value = expression
            .evaluate_as(dimension, &|id| self.inputs.parameters.value(id))
            .map_err(|error| {
                Failure::from(FeatureError::unevaluable(
                    self.feature.id(),
                    what,
                    expected,
                    error,
                ))
            })?;
        if value > 0.0 {
            Ok(value)
        } else {
            Err(self.error(BlendCause::NotPositive {
                what: what.to_owned(),
            }))
        }
    }

    fn shape(&self) -> Result<BlendShape, Failure> {
        let definition = self.definition;
        let size = self.value(
            &definition.size,
            Dimension::LENGTH,
            definition.kind.size_name(),
        )?;
        let flipped = definition.flipped;
        Ok(match (definition.kind, definition.chamfer_form()) {
            (BlendKind::Fillet, _) => BlendShape::Fillet { radius: size },
            (BlendKind::Chamfer, ChamferForm::Equal) => BlendShape::Chamfer { distance: size },
            (BlendKind::Chamfer, ChamferForm::TwoDistances { second }) => {
                BlendShape::TwoDistanceChamfer {
                    first: size,
                    second: self.value(second, Dimension::LENGTH, "second distance")?,
                    flipped,
                }
            }
            (BlendKind::Chamfer, ChamferForm::DistanceAngle { angle }) => {
                let degrees = self.value(angle, Dimension::ANGLE, "angle")?;
                if degrees >= 180.0 {
                    return Err(self.error(BlendCause::AngleTooWide));
                }
                BlendShape::AngledChamfer {
                    distance: size,
                    angle: degrees.to_radians(),
                    flipped,
                }
            }
        })
    }

    fn failure(&self, solid: &Solid, error: &BlendError) -> Failure {
        let kind = self.definition.kind;
        let edge = error.edge().map_or_else(
            || "an edge".to_owned(),
            |edge| describe_edge(self.inputs.document, solid, edge),
        );
        match error {
            BlendError::Cancelled(_) => Failure::Cancelled,
            BlendError::InvalidSize => self.error(BlendCause::SizeTooSmall { kind }),
            BlendError::InvalidAngle => self.error(BlendCause::AngleOutOfRange),
            BlendError::AngleMisses(_) => self.error(BlendCause::AngleMisses { edge }),
            BlendError::NoEdges => Failure::Waiting(MissingInput::Edges {
                body: self.definition.body,
            }),
            BlendError::MissingEdge(_) => self.error(BlendCause::EdgeGone {
                body: self.body_name.clone(),
            }),
            BlendError::Unsupported(_) => self.error(BlendCause::Unsupported { kind, edge }),
            BlendError::Intricate(_) => self.error(BlendCause::Intricate { kind, edge }),
            BlendError::Smooth(_) => self.error(BlendCause::Smooth { edge }),
            BlendError::TooLarge(_) => self.error(BlendCause::TooLarge { kind, edge }),
            BlendError::BackToBack(_) => self.error(BlendCause::BackToBack { kind, edge }),
            BlendError::WrapsAround(_) => self.error(BlendCause::WrapsAround { kind, edge }),
            BlendError::UnsupportedEnd { vertex, .. } => {
                match vertex.and_then(|vertex| self.earlier_blend_at(solid, vertex)) {
                    Some(earlier) => self.error(BlendCause::CornerBlended {
                        kind,
                        edge,
                        earlier,
                        feature: self.feature.name.clone(),
                    }),
                    None => self.error(BlendCause::EndUnsupported { kind, edge }),
                }
            }
            BlendError::Lost(_) => self.error(BlendCause::Lost { kind, edge }),
            BlendError::AfterFill(inner) => {
                log::warn!("{} could not be built: {inner}", self.feature.name);
                self.error(BlendCause::AfterFill {
                    kind,
                    body: self.body_name.clone(),
                })
            }
            BlendError::Profile { .. }
            | BlendError::Sweep { .. }
            | BlendError::Boolean { .. }
            | BlendError::Loft { .. }
            | BlendError::Rounding { .. } => {
                log::warn!("{} could not be built: {error}", self.feature.name);
                self.error(BlendCause::Unbuilt {
                    kind,
                    edge: error.edge().map(|_| edge),
                    body: self.body_name.clone(),
                })
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum BlendCause {
    NotPositive {
        what: String,
    },
    AngleTooWide,
    SizeTooSmall {
        kind: BlendKind,
    },
    AngleOutOfRange,
    AngleMisses {
        edge: String,
    },
    EdgeGone {
        body: String,
    },
    Unsupported {
        kind: BlendKind,
        edge: String,
    },
    Intricate {
        kind: BlendKind,
        edge: String,
    },
    Smooth {
        edge: String,
    },
    TooLarge {
        kind: BlendKind,
        edge: String,
    },
    BackToBack {
        kind: BlendKind,
        edge: String,
    },
    WrapsAround {
        kind: BlendKind,
        edge: String,
    },
    CornerBlended {
        kind: BlendKind,
        edge: String,
        earlier: String,
        feature: String,
    },
    EndUnsupported {
        kind: BlendKind,
        edge: String,
    },
    Lost {
        kind: BlendKind,
        edge: String,
    },
    AfterFill {
        kind: BlendKind,
        body: String,
    },
    Unbuilt {
        kind: BlendKind,
        edge: Option<String>,
        body: String,
    },
    PartlyFollowed {
        count: usize,
    },
}

impl BlendCause {
    pub(crate) fn reason(&self) -> String {
        match self {
            Self::NotPositive { what } => format!("The {what} must be more than zero."),
            Self::AngleTooWide => "The angle must be less than 180 degrees.".to_owned(),
            Self::SizeTooSmall { kind } => {
                format!("The {} must be more than 0.000001 mm.", kind.size_name())
            }
            Self::AngleOutOfRange => "The angle must be between 0 and 180 degrees.".to_owned(),
            Self::AngleMisses { edge } => {
                format!("At this angle the cut never meets the other face next to {edge}.")
            }
            Self::EdgeGone { body } => {
                format!("A chosen edge is no longer part of the body of {body}.")
            }
            Self::Unsupported { kind, edge } => format!(
                "A {} cannot follow {edge}: the faces there do not meet along one edge on each \
                 side, or the edge turns from an outer corner to an inner one.",
                kind.noun()
            ),
            Self::Intricate { kind, edge } => format!(
                "The faces next to {edge} curve too sharply for the {} to follow.",
                kind.noun()
            ),
            Self::Smooth { edge } => {
                format!("The faces meet smoothly at {edge}, so there is no corner to blend.")
            }
            Self::TooLarge { kind, edge } => format!(
                "The {} is too large for the faces next to {edge}.",
                kind.size_name()
            ),
            Self::BackToBack { kind, edge } => format!(
                "The faces at {edge} meet back to back, as two holes that touch do, so no {} of \
                 any size fits there.",
                kind.noun()
            ),
            Self::WrapsAround { kind, edge } => format!(
                "The {} of {edge} would run into itself: the edge is almost a full circle and \
                 its ends need room beyond it.",
                kind.noun()
            ),
            Self::CornerBlended {
                kind,
                edge,
                earlier,
                ..
            } => format!(
                "The {} cannot be closed off where {edge} ends. {earlier} already blended the \
                 corner there.",
                kind.noun()
            ),
            Self::EndUnsupported { kind, edge } => format!(
                "The {} cannot be closed off where {edge} ends.",
                kind.noun()
            ),
            Self::Lost { kind, edge } => format!(
                "The {} of {edge} could not be finished after the inner corners next to it were \
                 filled.",
                kind.noun()
            ),
            Self::AfterFill { kind, body } => format!(
                "The {} could not be finished after the inner corners of the body of {body} were \
                 filled.",
                kind.noun()
            ),
            Self::Unbuilt {
                kind,
                edge: Some(edge),
                ..
            } => format!("The {} of {edge} could not be built.", kind.noun()),
            Self::Unbuilt {
                kind,
                edge: None,
                body,
            } => format!(
                "The {} could not be built on the body of {body}.",
                kind.noun()
            ),
            Self::PartlyFollowed { count: 1 } => {
                "A chosen edge was split into pieces by a sketch edit, and only some of the \
                 pieces can still be traced back to it."
                    .to_owned()
            }
            Self::PartlyFollowed { count } => format!(
                "{count} chosen edges were split into pieces by a sketch edit, and only some of \
                 the pieces can still be traced back to them."
            ),
        }
    }

    pub(crate) fn remedy(&self) -> String {
        match self {
            Self::NotPositive { what } => format!("Enter a {what} above zero."),
            Self::AngleTooWide | Self::AngleOutOfRange => {
                "Enter an angle between 0 and 180 degrees, such as 45 deg.".to_owned()
            }
            Self::SizeTooSmall { kind } => format!("Enter a larger {}.", kind.size_name()),
            Self::AngleMisses { .. } => {
                "Enter a smaller angle, flip the chamfer to measure from the other face, or \
                 leave this edge out."
                    .to_owned()
            }
            Self::EdgeGone { .. } => {
                "Choose the edges again, or undo the change that removed it.".to_owned()
            }
            Self::Unsupported { .. } => {
                "Blend the outer and inner parts of the edge in separate features, or leave this \
                 edge out."
                    .to_owned()
            }
            Self::Intricate { kind, .. } | Self::TooLarge { kind, .. } => {
                format!(
                    "Enter a smaller {}, or leave this edge out.",
                    kind.size_name()
                )
            }
            Self::Smooth { .. } | Self::BackToBack { .. } => "Leave this edge out.".to_owned(),
            Self::WrapsAround { kind, .. } => format!(
                "Also choose the edges that continue from it, enter a smaller {}, or leave it \
                 out.",
                kind.size_name()
            ),
            Self::CornerBlended {
                earlier, feature, ..
            } => format!(
                "Move {feature} above {earlier} in the tree, so the corner is blended after \
                 this edge, or leave this edge out."
            ),
            Self::EndUnsupported { .. } => {
                "Also choose the edges that continue from it, or leave it out.".to_owned()
            }
            Self::Lost { .. } | Self::AfterFill { .. } => {
                "Blend the inner and the outer edges in separate features.".to_owned()
            }
            Self::Unbuilt {
                kind,
                edge: Some(_),
                ..
            } => format!(
                "Change the {} slightly, or leave this edge out.",
                kind.size_name()
            ),
            Self::Unbuilt {
                kind, edge: None, ..
            } => format!(
                "Change the {} slightly, or choose fewer edges.",
                kind.size_name()
            ),
            Self::PartlyFollowed { .. } => {
                "Choose the pieces of the edge again, or undo the sketch edit that split it."
                    .to_owned()
            }
        }
    }
}

impl From<BlendCause> for Cause {
    fn from(cause: BlendCause) -> Self {
        Self::Blend(cause)
    }
}

pub(crate) fn evaluate(
    feature: &Feature,
    definition: &Blend,
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
    let shape = context.shape()?;
    let Some(solid) = inputs.body(definition.body) else {
        return Err(inputs.missing_body(definition.body));
    };
    let edges = definition
        .resolve(inputs.document, solid)
        .map_err(|unresolved| {
            let body = &context.body_name;
            let (unresolved, chosen) = match unresolved {
                BlendUnresolved::Edges(unresolved) => (unresolved, Chosen::Edges),
                BlendUnresolved::Faces(unresolved) => (unresolved, Chosen::Faces),
                BlendUnresolved::PartlyFollowed(count) => {
                    return context.error(BlendCause::PartlyFollowed { count });
                }
            };
            Failure::from(
                FeatureError::new(Unmatched {
                    chosen,
                    unresolved,
                    body: body.clone(),
                })
                .fixing(FixTarget::Feature(feature.id())),
            )
        })?;
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let result = blend(solid, &edges, shape, feature.id().raw())
        .map_err(|error| context.failure(solid, &error))?;
    let recipe = BlendRecipe::of(feature.id(), shape, solid, &edges);
    Ok(FeatureResult::Solid(
        SolidResult::new(definition.body, result).blending(recipe),
    ))
}
