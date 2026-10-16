use caditor_document::{
    AxisReference, CurveStation, DatumKind, Document, Evaluation, FeatureId, PlaneReference,
    PointReference, PrincipalAxis, PrincipalPlane, SolidResult,
};
use caditor_expression::Expression;
use caditor_geometry::{Point2, Point3};
use caditor_kernel::{EdgeId, EdgeName, EdgeReference, FaceReference, Solid, vertex_names};
use serde::Deserialize;

use crate::{
    bodies, hole_tools,
    mcp::{
        references::{self, EntityName, Named, body_label},
        tools::ToolError,
        writing::PlaneName,
    },
    sketch_placement::{self, FaceChoice},
};

const EDGE_KEY_DIGITS: usize = 32;
const CORNER_TOLERANCE: f64 = 1e-6;
const NOT_FOUND_AT_THE_BAR: &str = "is not found where the new feature sits in the tree";

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PlaceError {
    #[error("“{0}” is not an edge key; use one list_edges gives")]
    UnreadableEdgeKey(String),
    #[error("{body} has no edge {edge} now; list_edges names its edges as they are")]
    NoEdge { body: String, edge: String },
    #[error("{what} cannot be used as a plane: {reason}")]
    NotAPlane { what: String, reason: String },
    #[error(
        "{what} has no axis: give a straight or round edge, a cylinder, cone or other round \
         face, a sketch line, a datum axis or X, Y or Z"
    )]
    NoAxis { what: String },
    #[error("{name} is not a datum {kind}; list_features names each datum")]
    NotADatum { name: String, kind: &'static str },
    #[error("{name} comes after the end of the tree, so a new feature cannot use it")]
    AfterTheBar { name: String },
    #[error(
        "{what} has no centre to use as a point: give a corner, a round edge, a sphere or torus, \
         a sketch point, a datum point, a coordinate system or origin"
    )]
    NoCentre { what: String },
    #[error(
        "{body} has no corner at [{x}, {y}, {z}]; list_edges gives the ends of its straight edges"
    )]
    NoCorner {
        body: String,
        x: f64,
        y: f64,
        z: f64,
    },
    #[error("{name} is not a point of {sketch}; get_sketch lists its points")]
    NotAPoint { name: String, sketch: String },
    #[error("face {face} of {body} {reason}")]
    UnusableFace {
        body: String,
        face: String,
        reason: &'static str,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum AxisName {
    X,
    Y,
    Z,
}

impl AxisName {
    pub(super) fn principal(self) -> PrincipalAxis {
        match self {
            Self::X => PrincipalAxis::X,
            Self::Y => PrincipalAxis::Y,
            Self::Z => PrincipalAxis::Z,
        }
    }
}

pub fn principal_plane(name: PlaneName) -> PrincipalPlane {
    match name {
        PlaneName::Xy => PrincipalPlane::Xy,
        PlaneName::Xz => PrincipalPlane::Xz,
        PlaneName::Yz => PrincipalPlane::Yz,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OnFace {
    pub body: Named,
    pub face: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OnEdge {
    pub body: Named,
    pub edge: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SketchLine {
    pub sketch: Named,
    pub line: EntityName,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatumNamed {
    pub datum: Named,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FramePlane {
    pub frame: Named,
    pub plane: PlaneName,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameAxis {
    pub frame: Named,
    pub axis: AxisName,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum PlaneChoice {
    Principal(PlaneName),
    Face(OnFace),
    Datum(DatumNamed),
    Frame(FramePlane),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum AxisChoice {
    Principal(AxisName),
    Edge(OnEdge),
    Face(OnFace),
    Sketch(SketchLine),
    Datum(DatumNamed),
    Frame(FrameAxis),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum OriginName {
    #[serde(rename = "origin")]
    Origin,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OnCorner {
    pub body: Named,
    pub corner: [f64; 3],
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SketchPoint {
    pub sketch: Named,
    pub point: EntityName,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum PointChoice {
    Origin(OriginName),
    Corner(OnCorner),
    Edge(OnEdge),
    Face(OnFace),
    Sketch(SketchPoint),
    Datum(DatumNamed),
}

pub fn edge_key_text(name: EdgeName) -> String {
    format!("{:0width$x}", name.digest(), width = EDGE_KEY_DIGITS)
}

pub fn edge_key(text: &str) -> Result<EdgeName, PlaceError> {
    let unreadable = || PlaceError::UnreadableEdgeKey(text.to_owned());
    let digits = text.trim();
    if digits.is_empty() || digits.len() > EDGE_KEY_DIGITS {
        return Err(unreadable());
    }
    let digest = u128::from_str_radix(digits, 16).map_err(|_| unreadable())?;
    Ok(EdgeName::from_digest(digest))
}

pub fn edge<'e>(
    document: &Document,
    evaluation: &'e Evaluation,
    body: FeatureId,
    text: &str,
) -> Result<(EdgeName, EdgeId, &'e SolidResult), ToolError> {
    let name = edge_key(text)?;
    let missing = || PlaceError::NoEdge {
        body: body_label(document, body),
        edge: text.to_owned(),
    };
    let shown = bodies::shown(evaluation, body).ok_or_else(missing)?;
    let found = bodies::find_edge(shown, name).ok_or_else(missing)?;
    Ok((name, found, shown))
}

fn datum(
    document: &Document,
    named: &Named,
    kind: DatumKind,
    word: &'static str,
) -> Result<FeatureId, ToolError> {
    datum_of(document, named, &[kind], word).map(|(feature, _)| feature)
}

fn datum_of(
    document: &Document,
    named: &Named,
    kinds: &[DatumKind],
    word: &'static str,
) -> Result<(FeatureId, DatumKind), ToolError> {
    let found = references::feature(document, named)?;
    let Some(kind) = found
        .kind
        .datum()
        .map(|datum| datum.kind())
        .filter(|kind| kinds.contains(kind))
    else {
        return Err(PlaceError::NotADatum {
            name: found.name.clone(),
            kind: word,
        }
        .into());
    };
    before_the_bar(document, found.id(), &found.name)?;
    Ok((found.id(), kind))
}

fn before_the_bar(document: &Document, feature: FeatureId, name: &str) -> Result<(), PlaceError> {
    let before = document
        .feature_index(feature)
        .is_some_and(|index| index < document.bar_index());
    if before {
        Ok(())
    } else {
        Err(PlaceError::AfterTheBar {
            name: name.to_owned(),
        })
    }
}

fn frame(document: &Document, named: &Named) -> Result<FeatureId, ToolError> {
    datum(document, named, DatumKind::Frame, "coordinate system")
}

pub fn plane(
    document: &Document,
    evaluation: &Evaluation,
    choice: &PlaneChoice,
) -> Result<PlaneReference, ToolError> {
    match choice {
        PlaneChoice::Principal(name) => Ok(PlaneReference::Principal(principal_plane(*name))),
        PlaneChoice::Datum(named) => {
            datum(document, &named.datum, DatumKind::Plane, "plane").map(PlaneReference::Datum)
        }
        PlaneChoice::Face(on) => on_face(document, evaluation, on).map(|placed| placed.reference),
        PlaneChoice::Frame(on) => Ok(PlaneReference::Frame {
            frame: frame(document, &on.frame)?,
            plane: principal_plane(on.plane),
        }),
    }
}

pub struct FacePlace {
    pub reference: PlaneReference,
    pub middle: Option<Point2>,
}

pub fn on_face(
    document: &Document,
    evaluation: &Evaluation,
    on: &OnFace,
) -> Result<FacePlace, ToolError> {
    let body = references::body(document, evaluation, &on.body)?;
    let (key, face, shown) = references::face(document, evaluation, body, &on.face)?;
    let (attachment, frame) = sketch_placement::attachment_in(
        document,
        evaluation,
        FaceChoice { body, face: key },
        document.bar_index(),
    )
    .map_err(|reason| PlaceError::NotAPlane {
        what: format!("face {} of {}", on.face, body_label(document, body)),
        reason: reason.to_string(),
    })?;
    Ok(FacePlace {
        reference: PlaneReference::Face(attachment),
        middle: hole_tools::middle_of(&shown.solid, face, &frame),
    })
}

pub fn axis(
    document: &Document,
    evaluation: &Evaluation,
    choice: &AxisChoice,
) -> Result<AxisReference, ToolError> {
    let index = document.bar_index();
    match choice {
        AxisChoice::Principal(name) => Ok(AxisReference::Principal(name.principal())),
        AxisChoice::Datum(named) => {
            datum(document, &named.datum, DatumKind::Axis, "axis").map(AxisReference::Datum)
        }
        AxisChoice::Edge(on) => {
            let body = references::body(document, evaluation, &on.body)?;
            let (_, found, shown) = edge(document, evaluation, body, &on.edge)?;
            let no_axis = || PlaceError::NoAxis {
                what: format!("edge {} of {}", on.edge, body_label(document, body)),
            };
            let reference = EdgeReference::capture(&shown.solid, found).ok_or_else(no_axis)?;
            let state = sketch_placement::body_state_in(document, evaluation, body, index)
                .map_err(|_| no_axis())?;
            let there = reference.resolve(state).map_err(|_| no_axis())?;
            Ok(AxisReference::capture_edge(body, state, there).ok_or_else(no_axis)?)
        }
        AxisChoice::Face(on) => {
            let body = references::body(document, evaluation, &on.body)?;
            let (_, found, shown) = references::face(document, evaluation, body, &on.face)?;
            let no_axis = || PlaceError::NoAxis {
                what: format!("face {} of {}", on.face, body_label(document, body)),
            };
            let reference = FaceReference::capture(&shown.solid, found).ok_or_else(no_axis)?;
            let state = sketch_placement::body_state_in(document, evaluation, body, index)
                .map_err(|_| no_axis())?;
            let there = reference.resolve(state).map_err(|_| no_axis())?;
            Ok(AxisReference::capture_face(body, state, there).ok_or_else(no_axis)?)
        }
        AxisChoice::Sketch(line) => {
            let (feature, sketch) = references::sketch(document, &line.sketch)?;
            let entity = references::entity(&feature.name, sketch, &line.line)?;
            if sketch.line_endpoints(entity).is_none() {
                return Err(PlaceError::NoAxis {
                    what: format!("entity {} of {}", line.line, feature.name),
                }
                .into());
            }
            Ok(AxisReference::Sketch {
                sketch: feature.id(),
                entity,
            })
        }
        AxisChoice::Frame(on) => Ok(AxisReference::Frame {
            frame: frame(document, &on.frame)?,
            axis: on.axis.principal(),
        }),
    }
}

pub struct AtTheBar<'e> {
    pub body: FeatureId,
    pub state: &'e Solid,
}

fn state_at_bar<'e>(
    document: &Document,
    evaluation: &'e Evaluation,
    body: FeatureId,
    lost: impl Fn() -> PlaceError,
) -> Result<AtTheBar<'e>, PlaceError> {
    sketch_placement::body_state_in(document, evaluation, body, document.bar_index())
        .map(|state| AtTheBar { body, state })
        .map_err(|_| lost())
}

pub fn face_at_bar<'e>(
    document: &Document,
    evaluation: &'e Evaluation,
    on: &OnFace,
) -> Result<(AtTheBar<'e>, FaceReference), ToolError> {
    let body = references::body(document, evaluation, &on.body)?;
    let (_, found, shown) = references::face(document, evaluation, body, &on.face)?;
    let lost = || PlaceError::UnusableFace {
        body: body_label(document, body),
        face: on.face.clone(),
        reason: NOT_FOUND_AT_THE_BAR,
    };
    let seen = FaceReference::capture(&shown.solid, found).ok_or_else(lost)?;
    let at = state_at_bar(document, evaluation, body, lost)?;
    let there = seen.resolve(at.state).map_err(|_| lost())?;
    let reference = FaceReference::capture(at.state, there).ok_or_else(lost)?;
    Ok((at, reference))
}

pub fn edge_at_bar<'e>(
    document: &Document,
    evaluation: &'e Evaluation,
    on: &OnEdge,
) -> Result<(AtTheBar<'e>, EdgeId), ToolError> {
    let body = references::body(document, evaluation, &on.body)?;
    let (_, found, shown) = edge(document, evaluation, body, &on.edge)?;
    let lost = || PlaceError::NoEdge {
        body: body_label(document, body),
        edge: on.edge.clone(),
    };
    let seen = EdgeReference::capture(&shown.solid, found).ok_or_else(lost)?;
    let at = state_at_bar(document, evaluation, body, lost)?;
    let there = seen.resolve(at.state).map_err(|_| lost())?;
    Ok((at, there))
}

pub fn station(
    document: &Document,
    evaluation: &Evaluation,
    on: &OnEdge,
    distance: Expression,
) -> Result<CurveStation, ToolError> {
    let (at, edge) = edge_at_bar(document, evaluation, on)?;
    CurveStation::capture(at.body, at.state, edge, distance).ok_or_else(|| {
        PlaceError::NoEdge {
            body: body_label(document, at.body),
            edge: on.edge.clone(),
        }
        .into()
    })
}

fn corner(
    document: &Document,
    evaluation: &Evaluation,
    on: &OnCorner,
) -> Result<PointReference, ToolError> {
    let body = references::body(document, evaluation, &on.body)?;
    let [x, y, z] = on.corner;
    let wanted = Point3::new(x, y, z);
    let missing = || PlaceError::NoCorner {
        body: body_label(document, body),
        x,
        y,
        z,
    };
    let shown = bodies::shown(evaluation, body).ok_or_else(missing)?;
    let reach = CORNER_TOLERANCE * wanted.length().max(1.0);
    let found = shown
        .solid
        .vertices()
        .map(|(id, vertex)| (id, vertex.point().distance(wanted)))
        .filter(|(_, apart)| *apart <= reach)
        .min_by(|one, other| one.1.total_cmp(&other.1))
        .map(|(id, _)| id)
        .ok_or_else(missing)?;
    let name = vertex_names(&shown.solid)
        .get(&found)
        .copied()
        .ok_or_else(missing)?;
    let at = state_at_bar(document, evaluation, body, missing)?;
    let named = vertex_names(at.state)
        .values()
        .filter(|there| **there == name)
        .count();
    if named != 1 {
        return Err(missing().into());
    }
    Ok(PointReference::Vertex { body, vertex: name })
}

pub fn point(
    document: &Document,
    evaluation: &Evaluation,
    choice: &PointChoice,
) -> Result<PointReference, ToolError> {
    match choice {
        PointChoice::Origin(OriginName::Origin) => Ok(PointReference::Origin),
        PointChoice::Datum(named) => match datum_of(
            document,
            &named.datum,
            &[DatumKind::Point, DatumKind::Frame],
            "point or coordinate system",
        )? {
            (feature, DatumKind::Frame) => Ok(PointReference::Frame(feature)),
            (feature, _) => Ok(PointReference::Datum(feature)),
        },
        PointChoice::Corner(on) => corner(document, evaluation, on),
        PointChoice::Edge(on) => {
            let (at, edge) = edge_at_bar(document, evaluation, on)?;
            PointReference::capture_centre(at.body, at.state, edge).ok_or_else(|| {
                PlaceError::NoCentre {
                    what: format!("edge {} of {}", on.edge, body_label(document, at.body)),
                }
                .into()
            })
        }
        PointChoice::Face(on) => {
            let (at, face) = face_at_bar(document, evaluation, on)?;
            let no_centre = || PlaceError::NoCentre {
                what: format!("face {} of {}", on.face, body_label(document, at.body)),
            };
            let there = face.resolve(at.state).map_err(|_| no_centre())?;
            PointReference::capture_surface_centre(at.body, at.state, there)
                .ok_or_else(|| no_centre().into())
        }
        PointChoice::Sketch(on) => {
            let (feature, sketch) = references::sketch(document, &on.sketch)?;
            let entity = references::entity(&feature.name, sketch, &on.point)?;
            if sketch.point(entity).is_none() {
                return Err(PlaceError::NotAPoint {
                    name: on.point.to_string(),
                    sketch: feature.name.clone(),
                }
                .into());
            }
            before_the_bar(document, feature.id(), &feature.name)?;
            Ok(PointReference::Sketch {
                sketch: feature.id(),
                entity,
            })
        }
    }
}
